// Copyright 2025 The Rustux Authors
//
// xHCI Controller - Transfer Implementation
//
// This module implements xHCI controller driver with transfer ring and event ring support.

use crate::usb::{XhciInfo, UsbError, trb::*};
use core::sync::atomic::{AtomicU32, Ordering};

/// Maximum number of USB devices
const MAX_DEVICES: usize = 8;

/// xHCI Capability Register Length (CAPLENGTH)
const REG_CAPLENGTH: u8 = 0x00;

/// xHCI Operational Registers
const REG_USBCMD: usize = 0x000;
const REG_USBSTS: usize = 0x004;
const REG_PAGESIZE: usize = 0x008;
const REG_DNCTRL: usize = 0x014;
const REG_CRCR: usize = 0x018;
const REG_DCBAAP: usize = 0x030;
const REG_CONFIG: usize = 0x038;

/// Runtime register offsets
const REG_MFINDEX: usize = 0x000;
const REG_ERDP_LO: usize = 0x020; // Event Ring Dequeue Pointer
const REG_ERDP_HI: usize = 0x024;

/// Port register set base
const REG_PORTSC_BASE: usize = 0x400;

/// USB command register bits
const USBCMD_RUN: u32 = 0x0000_0001;
const USBCMD_HCRST: u32 = 0x0000_0002;

/// USB status register bits
const USBSTS_HCH: u32 = 0x0000_1000; // HCHalted
const USBSTS_CNR: u32 = 0x0000_0800; // Controller Not Ready
const USBSTS_EINT: u32 = 0x0000_0004; // Event Interrupt
const USBSTS_PCD: u32 = 0x0000_0002; // Port Change Detect
const USBSTS_HSE: u32 = 0x0000_0010; // Host System Error

/// Port status register bits
pub const PORTSC_CCS: u32 = 0x0000_0001; // Current Connect Status
const PORTSC_CSC: u32 = 0x0000_0002; // Connect Status Change
const PORTSC_PED: u32 = 0x0000_0002; // Port Enabled/Disabled
const PORTSC_PEDC: u32 = 0x0000_0010; // Port Enabled/Disabled Change
const PORTSC_OCA: u32 = 0x0000_0020; // Over-current Active
const PORTSC_PR: u32 = 0x0000_0010; // Port Reset
const PORTSC_PLS_MASK: u32 = 0x0000_01E0; // Port Link State
const PORTSC_PLS_U0: u32 = 0x0000_0000; // U0 state
const PORTSC_PP: u32 = 0x0000_0200; // Port Power
const PORTSC_PORT_SPEED_MASK: u32 = 0x0000_F000; // Port Speed
const PORTSC_PORT_SPEED_SHIFT: u32 = 12;

/// Transfer Ring
#[repr(C)]
pub struct TransferRing {
    /// TRB data (16 bytes each, aligned to 64-byte boundary)
    pub trbs: [NormalTrb; TRB_RING_SIZE],
    /// Enqueue pointer (index)
    pub enqueue: AtomicU32,
    /// Dequeue pointer (index)
    pub dequeue: AtomicU32,
    /// Cycle state (0 or 1)
    pub cycle_state: AtomicU32,
}

impl TransferRing {
    /// Create a new transfer ring
    pub fn new() -> Self {
        Self {
            trbs: [NormalTrb { data_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
            enqueue: AtomicU32::new(0),
            dequeue: AtomicU32::new(0),
            cycle_state: AtomicU32::new(1),
        }
    }

    /// Enqueue a TRB
    pub unsafe fn enqueue(&self, trb: &NormalTrb) -> Result<(), UsbError> {
        let index = self.enqueue.load(Ordering::Acquire) as usize;
        let cycle = self.cycle_state.load(Ordering::Acquire);

        // Check if ring is full
        let next_index = (index + 1) % TRB_RING_SIZE;
        if next_index == self.dequeue.load(Ordering::Acquire) as usize {
            return Err(UsbError::Timeout); // Ring full
        }

        // Write TRB
        let trb_ptr = &self.trbs[index] as *const NormalTrb as *mut NormalTrb;
        (*trb_ptr).data_ptr = trb.data_ptr;
        (*trb_ptr).status = trb.status;
        (*trb_ptr).control = trb.control | (cycle & 1);

        // Update enqueue pointer
        self.enqueue.store(next_index as u32, Ordering::Release);

        // Toggle cycle bit if wrapping
        if next_index == 0 {
            let new_cycle = cycle ^ 1;
            self.cycle_state.store(new_cycle, Ordering::Release);
        }

        Ok(())
    }

    /// Get enqueue pointer for doorbell
    pub fn enqueue_ptr(&self) -> u64 {
        &self.trbs[0] as *const NormalTrb as u64
    }
}

/// Event Ring
#[repr(C)]
pub struct EventRing {
    /// Event TRBs (16 bytes each, aligned to 64-byte boundary)
    pub trbs: [EventTrb; TRB_RING_SIZE],
    /// Dequeue pointer (index)
    pub dequeue: AtomicU32,
    /// Cycle state (0 or 1)
    pub cycle_state: AtomicU32,
}

impl EventRing {
    /// Create a new event ring
    pub fn new() -> Self {
        Self {
            trbs: [EventTrb { trb_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
            dequeue: AtomicU32::new(0),
            cycle_state: AtomicU32::new(1),
        }
    }

    /// Dequeue an event TRB
    pub unsafe fn dequeue(&self) -> Option<EventTrb> {
        let index = self.dequeue.load(Ordering::Acquire) as usize;
        let cycle = self.cycle_state.load(Ordering::Acquire);

        let trb = self.trbs[index];
        let trb_cycle = (trb.control & TRB_CYCLE_BIT) as u32;

        // Check if TRB is owned by controller (cycle bit mismatch)
        if trb_cycle != (cycle & 1) {
            return None; // No new events
        }

        // Update dequeue pointer
        let next_index = (index + 1) % TRB_RING_SIZE;
        self.dequeue.store(next_index as u32, Ordering::Release);

        // Toggle cycle bit if wrapping
        if next_index == 0 {
            let new_cycle = cycle ^ 1;
            self.cycle_state.store(new_cycle, Ordering::Release);
        }

        Some(trb)
    }

    /// Get dequeue pointer for ERDP
    pub fn dequeue_ptr(&self) -> u64 {
        &self.trbs[0] as *const EventTrb as u64 + (self.dequeue.load(Ordering::Acquire) as u64 * 16)
    }
}

/// xHCI controller state
pub struct XhciController {
    mmio_base: u64,
    cap_length: u8,
    runtime_offset: u64,
    doorbell_offset: u64,
    pub transfer_ring: TransferRing,
    pub event_ring: EventRing,
}

impl XhciController {
    /// Initialize xHCI controller from MMIO base
    pub unsafe fn new(info: XhciInfo) -> Result<Self, UsbError> {
        let mmio_base = info.mmio_base as *mut u8;

        crate::framebuffer::write_str("xHCI: Reading CAPLENGTH...\n");

        // Read CAPLENGTH to get capability register length
        let cap_length = mmio_base.add(REG_CAPLENGTH as usize).read_volatile() as u8;

        // Verify MMIO is working - check for 0xFF (unmapped memory)
        if cap_length == 0xFF {
            crate::framebuffer::write_str("ERROR: CAPLENGTH = 0xFF - MMIO not mapped!\n");
            return Err(UsbError::XhciInitFailed);
        }

        crate::framebuffer::write_str("xHCI: CAPLENGTH = ");
        // Simple decimal print
        if cap_length < 10 {
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + cap_length]).unwrap());
        } else {
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + (cap_length / 10)]).unwrap());
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + (cap_length % 10)]).unwrap());
        }
        crate::framebuffer::write_str("\n");

        // Read HCIVERSION (offset 0x02 in capability registers)
        let hciversion = mmio_base.add(0x02).cast::<u16>().read_volatile();

        crate::framebuffer::write_str("xHCI: HCIVERSION = 0x");
        let mut version = hciversion;
        for _ in 0..4 {
            let nibble = (version & 0xF000) >> 12;
            let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
            crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
            version <<= 4;
        }
        crate::framebuffer::write_str("\n");

        if hciversion < 0x0100 {
            crate::framebuffer::write_str("ERROR: Unsupported xHCI version (< 1.0)\n");
            return Err(UsbError::XhciInitFailed);
        }

        // Read RTSOFF to get runtime register offset (offset 0x18 in capability registers)
        // This is a 32-bit register, so we need to cast to u32 pointer
        let rts_offset = mmio_base.add(0x18).cast::<u32>().read_volatile() & 0xFFFF_FFFE;
        let runtime_offset = rts_offset as u64;

        // Read DBOFF to get doorbell offset (offset 0x1C in capability registers)
        // This is a 32-bit register, so we need to cast to u32 pointer
        let dboff = mmio_base.add(0x1C).cast::<u32>().read_volatile() & 0xFFFF_FFFE;
        let doorbell_offset = dboff as u64;

        crate::framebuffer::write_str("xHCI: RTSOFF = 0x");
        let mut rts = rts_offset;
        for _ in 0..8 {
            let nibble = (rts & 0xF0000000) >> 28;
            let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
            crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
            rts <<= 4;
        }
        crate::framebuffer::write_str("\n");

        crate::framebuffer::write_str("xHCI: Resetting controller...\n");

        let mut controller = Self {
            mmio_base: info.mmio_base,
            cap_length,
            runtime_offset,
            doorbell_offset,
            transfer_ring: TransferRing::new(),
            event_ring: EventRing::new(),
        };

        // Reset and initialize controller
        controller.reset()?;
        controller.init_operational()?;

        crate::framebuffer::write_str("xHCI: Controller ready\n");

        Ok(controller)
    }

    /// Read operational register
    unsafe fn read_op_reg(&self, offset: usize) -> u32 {
        let mmio_base = self.mmio_base as *mut u8;
        let addr = mmio_base.add(self.cap_length as usize + offset);
        addr.cast::<u32>().read_volatile()
    }

    /// Write operational register
    unsafe fn write_op_reg(&self, offset: usize, value: u32) {
        let mmio_base = self.mmio_base as *mut u8;
        let addr = mmio_base.add(self.cap_length as usize + offset);
        addr.cast::<u32>().write_volatile(value);
    }

    /// Read runtime register
    unsafe fn read_runtime_reg(&self, offset: usize) -> u32 {
        let mmio_base = self.mmio_base as *mut u8;
        let addr = mmio_base.add(self.runtime_offset as usize + offset);
        addr.cast::<u32>().read_volatile()
    }

    /// Write runtime register
    unsafe fn write_runtime_reg(&self, offset: usize, value: u32) {
        let mmio_base = self.mmio_base as *mut u8;
        let addr = mmio_base.add(self.runtime_offset as usize + offset);
        addr.cast::<u32>().write_volatile(value);
    }

    /// Read port status register
    pub unsafe fn read_port_sc(&self, port: usize) -> u32 {
        self.read_op_reg(REG_PORTSC_BASE + (port * 0x10))
    }

    /// Write port status register
    unsafe fn write_port_sc(&self, port: usize, value: u32) {
        self.write_op_reg(REG_PORTSC_BASE + (port * 0x10), value);
    }

    /// Reset xHCI controller
    unsafe fn reset(&mut self) -> Result<(), UsbError> {
        // Check if controller is halted
        let status = self.read_op_reg(REG_USBSTS);
        if status & USBSTS_HCH == 0 {
            // Controller running - issue reset
            self.write_op_reg(REG_USBCMD, USBCMD_HCRST);

            // Wait for reset to complete (HCRST bit clears and CNR sets)
            let mut timeout = 100000;
            while timeout > 0 {
                let cmd = self.read_op_reg(REG_USBCMD);
                let sts = self.read_op_reg(REG_USBSTS);

                if cmd & USBCMD_HCRST == 0 && sts & USBSTS_CNR == 0 {
                    return Ok(());
                }

                timeout -= 1;
                for _ in 0..100 {
                    core::arch::asm!("nop", options(nomem, nostack));
                }
            }

            return Err(UsbError::XhciInitFailed);
        }

        Ok(())
    }

    /// Initialize operational registers
    unsafe fn init_operational(&mut self) -> Result<(), UsbError> {
        // Set max device slots
        self.write_op_reg(REG_CONFIG, 8); // Max slots = 8

        // Start controller
        self.write_op_reg(REG_USBCMD, USBCMD_RUN);

        // Wait for controller to be ready
        let mut timeout = 100000;
        while timeout > 0 {
            let status = self.read_op_reg(REG_USBSTS);
            if status & USBSTS_CNR == 0 {
                break;
            }
            timeout -= 1;
            for _ in 0..100 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }

        if timeout == 0 {
            return Err(UsbError::XhciInitFailed);
        }

        Ok(())
    }

    /// Get MMIO base address
    pub fn mmio_base(&self) -> u64 {
        self.mmio_base
    }

    /// Check if controller is running
    pub fn is_running(&self) -> bool {
        unsafe {
            let status = self.read_op_reg(REG_USBSTS);
            status & USBSTS_HCH == 0
        }
    }

    /// Ring doorbell for an endpoint
    pub unsafe fn ring_doorbell(&self, slot: u8, endpoint: u8, stream: u16) {
        let db_offset = self.doorbell_offset + ((slot as u64) * 4);
        let mmio_base = self.mmio_base as *mut u8;
        let db_addr = mmio_base.add(db_offset as usize);
        let db_value: u32 = ((stream as u32) << 16) | ((endpoint as u32) & 0xFF);
        db_addr.cast::<u32>().write_volatile(db_value);
    }

    /// Check for port connection
    pub unsafe fn check_port_connection(&self, port: usize) -> bool {
        let portsc = self.read_port_sc(port);
        (portsc & PORTSC_CCS) != 0 && (portsc & PORTSC_PED) != 0
    }

    /// Reset a port
    pub unsafe fn reset_port(&self, port: usize) -> Result<(), UsbError> {
        let mut portsc = self.read_port_sc(port);

        // Set port reset bit
        self.write_port_sc(port, portsc | PORTSC_PR);

        // Wait for reset to complete
        let mut timeout = 100000;
        while timeout > 0 {
            portsc = self.read_port_sc(port);
            if portsc & PORTSC_PR == 0 {
                break;
            }
            timeout -= 1;
            for _ in 0..100 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }

        if timeout == 0 {
            return Err(UsbError::XhciInitFailed);
        }

        // Check if port is enabled
        portsc = self.read_port_sc(port);
        if portsc & PORTSC_PED == 0 {
            return Err(UsbError::DeviceNotFound);
        }

        Ok(())
    }

    /// Poll for events
    pub unsafe fn poll_events(&self) -> Option<EventTrb> {
        // Update ERDP to clear events
        let erdp = self.event_ring.dequeue_ptr();
        self.write_runtime_reg(REG_ERDP_LO, (erdp & 0xFFFF_FFFF) as u32);
        self.write_runtime_reg(REG_ERDP_HI, ((erdp >> 32) & 0xFFFF_FFFF) as u32);

        // Try to dequeue an event
        self.event_ring.dequeue()
    }
}

/// Global xHCI controller instance
static mut XHCI_CONTROLLER: Option<XhciController> = None;

/// Initialize xHCI controller
pub unsafe fn init() -> Result<(), UsbError> {
    let info = super::pci::scan_for_xhci()?;
    let controller = XhciController::new(info)?;
    XHCI_CONTROLLER = Some(controller);
    Ok(())
}

/// Get xHCI controller reference
pub unsafe fn controller() -> Option<&'static XhciController> {
    XHCI_CONTROLLER.as_ref()
}

/// Get xHCI controller mutable reference
pub unsafe fn controller_mut() -> Option<&'static mut XhciController> {
    XHCI_CONTROLLER.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(USBCMD_RUN, 0x0000_0001);
        assert_eq!(USBCMD_HCRST, 0x0000_0002);
        assert_eq!(USBSTS_HCH, 0x0000_1000);
        assert_eq!(USBSTS_CNR, 0x0000_0800);
    }

    #[test]
    fn test_ring_sizes() {
        assert_eq!(TRB_RING_SIZE, 16);
        assert_eq!(core::mem::size_of::<NormalTrb>(), 16);
        assert_eq!(core::mem::size_of::<EventTrb>(), 16);
    }
}
