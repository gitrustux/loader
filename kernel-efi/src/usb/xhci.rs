// Copyright 2025 The Rustux Authors
//
// xHCI Controller - Transfer Implementation
//
// This module implements xHCI controller driver with transfer ring and event ring support.

use crate::usb::{XhciInfo, UsbError, trb::*};
use crate::usb::dma::*;
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

/// Runtime register offsets (xHCI 5.5). MFINDEX is at 0x00; the register
/// set for interrupter 0 starts at 0x20.
const REG_IMAN: usize = 0x020; // Interrupter Management
const REG_IMOD: usize = 0x024; // Interrupter Moderation
const REG_ERSTSZ_0: usize = 0x028; // Event Ring Segment Table Size
const REG_ERSTBA_LO_0: usize = 0x030; // Event Ring Segment Table Base Address
const REG_ERSTBA_HI_0: usize = 0x034;
const REG_ERDP_LO_0: usize = 0x038; // Event Ring Dequeue Pointer
const REG_ERDP_HI_0: usize = 0x03C;

/// ERDP Event Handler Busy (RW1C)
const ERDP_EHB: u32 = 0x0000_0008;

/// Capability register offsets
const CAP_HCSPARAMS1: usize = 0x04;
const CAP_HCSPARAMS2: usize = 0x08;
const CAP_HCCPARAMS1: usize = 0x10;

/// Extended capability: USB Legacy Support (BIOS/OS handoff)
const XECP_ID_LEGACY: u32 = 1;
const USBLEGSUP_BIOS_OWNED: u32 = 1 << 16;
const USBLEGSUP_OS_OWNED: u32 = 1 << 24;

/// Interrupter Management bits
const IMAN_IP: u32 = 0x0000_0001; // Interrupt Pending
const IMAN_IE: u32 = 0x0000_0002; // Interrupt Enable

/// Port register set base
const REG_PORTSC_BASE: usize = 0x400;

/// USB command register bits
const USBCMD_RUN: u32 = 0x0000_0001;
const USBCMD_HCRST: u32 = 0x0000_0002;

/// USB status register bits (xHCI 5.4.2)
const USBSTS_HCH: u32 = 0x0000_0001; // Bit 0: HCHalted
const USBSTS_HSE: u32 = 0x0000_0004; // Bit 2: Host System Error
const USBSTS_EINT: u32 = 0x0000_0008; // Bit 3: Event Interrupt (RW1C)
const USBSTS_PCD: u32 = 0x0000_0010; // Bit 4: Port Change Detect (RW1C)
const USBSTS_CNR: u32 = 0x0000_0800; // Bit 11: Controller Not Ready
const USBSTS_HCE: u32 = 0x0000_1000; // Bit 12: Host Controller Error

/// Port status register bits (xHCI 5.4.8)
pub const PORTSC_CCS: u32 = 1 << 0; // Current Connect Status
pub const PORTSC_PED: u32 = 1 << 1; // Port Enabled (RW1CS: writing 1 DISABLES the port)
pub const PORTSC_PR: u32 = 1 << 4; // Port Reset
pub const PORTSC_PP: u32 = 1 << 9; // Port Power
pub const PORTSC_PORT_SPEED_MASK: u32 = 0xF << 10; // Port Speed (bits 13:10)
pub const PORTSC_PORT_SPEED_SHIFT: u32 = 10;
pub const PORTSC_PRC: u32 = 1 << 21; // Port Reset Change (RW1C)
/// All RW1C change bits (CSC, PEC, WRC, OCC, PRC, PLC, CEC)
pub const PORTSC_CHANGE_BITS: u32 = 0x00FE_0000;
/// Bits that keep their value when written back (PP, PIC, WCE/WDE/WOE).
/// Everything else must be written as 0 to avoid disabling the port or
/// clearing change bits by accident.
pub const PORTSC_PRESERVE: u32 = PORTSC_PP | (3 << 14) | (7 << 25);

/// Port speed IDs (default Protocol Speed IDs)
pub const SPEED_FULL: u32 = 1;
pub const SPEED_LOW: u32 = 2;
pub const SPEED_HIGH: u32 = 3;
pub const SPEED_SUPER: u32 = 4;

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

/// xHCI controller state
pub struct XhciController {
    mmio_base: u64,
    cap_length: u8,
    runtime_offset: u64,
    doorbell_offset: u64,
    max_ports: u8,
    max_slots: u8,
    scratchpad_count: u32,
    context_size_64: bool,
    // Rings are now in the dma module, not owned here
    pub transfer_ring: TransferRing,
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

        crate::framebuffer::write_str("xHCI: CAPLENGTH = 0x");
        print_hex_u32(cap_length as u32);
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

        // Structural / capability parameters
        let hcsparams1 = mmio_base.add(CAP_HCSPARAMS1).cast::<u32>().read_volatile();
        let hcsparams2 = mmio_base.add(CAP_HCSPARAMS2).cast::<u32>().read_volatile();
        let hccparams1 = mmio_base.add(CAP_HCCPARAMS1).cast::<u32>().read_volatile();
        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;
        let scratchpad_count = (((hcsparams2 >> 21) & 0x1F) << 5) | ((hcsparams2 >> 27) & 0x1F);
        let context_size_64 = hccparams1 & (1 << 2) != 0;

        crate::framebuffer::write_str("xHCI: MaxSlots=");
        print_decimal(max_slots as u32);
        crate::framebuffer::write_str(" MaxPorts=");
        print_decimal(max_ports as u32);
        crate::framebuffer::write_str(" Scratchpads=");
        print_decimal(scratchpad_count);
        crate::framebuffer::write_str(if context_size_64 { " CSZ=64\n" } else { " CSZ=32\n" });

        if context_size_64 {
            // Our Input/Device Context structs use the 32-byte layout
            crate::framebuffer::write_str("xHCI: ERROR - 64-byte contexts not supported yet\n");
            return Err(UsbError::XhciInitFailed);
        }

        // Take ownership from the BIOS before touching the controller
        bios_handoff(mmio_base, hccparams1);

        crate::framebuffer::write_str("xHCI: Resetting controller...\n");

        // Initialize ERST with event ring address
        unsafe {
            erst_init();
        }

        let mut controller = Self {
            mmio_base: info.mmio_base,
            cap_length,
            runtime_offset,
            doorbell_offset,
            max_ports,
            max_slots,
            scratchpad_count,
            context_size_64,
            transfer_ring: TransferRing::new(),
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

    /// Read port status register. `port` is 1-based (1..=max_ports), as in
    /// the xHCI spec and the Slot Context "Root Hub Port Number".
    pub unsafe fn read_port_sc(&self, port: usize) -> u32 {
        if port == 0 || port > self.max_ports as usize {
            return 0;
        }
        self.read_op_reg(REG_PORTSC_BASE + ((port - 1) * 0x10))
    }

    /// Write port status register. `port` is 1-based.
    pub unsafe fn write_port_sc(&self, port: usize, value: u32) {
        if port == 0 || port > self.max_ports as usize {
            return;
        }
        self.write_op_reg(REG_PORTSC_BASE + ((port - 1) * 0x10), value);
    }

    /// Number of root hub ports (HCSPARAMS1)
    pub fn max_ports(&self) -> usize {
        self.max_ports as usize
    }

    /// Reset xHCI controller (xHCI 4.2): stop it, wait for HCHalted, then
    /// issue HCRST and wait for HCRST and CNR to clear.
    unsafe fn reset(&mut self) -> Result<(), UsbError> {
        // Stop the controller if it is running
        let cmd = self.read_op_reg(REG_USBCMD);
        if cmd & USBCMD_RUN != 0 {
            self.write_op_reg(REG_USBCMD, cmd & !USBCMD_RUN);
        }
        if !self.wait_op(REG_USBSTS, USBSTS_HCH, USBSTS_HCH) {
            crate::framebuffer::write_str("xHCI: ERROR - controller did not halt\n");
            return Err(UsbError::XhciInitFailed);
        }

        self.write_op_reg(REG_USBCMD, USBCMD_HCRST);
        // Some Intel controllers need a short delay before HCRST can be read back
        spin(10_000);
        if !self.wait_op(REG_USBCMD, USBCMD_HCRST, 0) || !self.wait_op(REG_USBSTS, USBSTS_CNR, 0) {
            crate::framebuffer::write_str("xHCI: ERROR - reset did not complete\n");
            return Err(UsbError::XhciInitFailed);
        }
        crate::framebuffer::write_str("xHCI: Reset OK\n");
        Ok(())
    }

    /// Poll an operational register until (value & mask) == expected.
    unsafe fn wait_op(&self, reg: usize, mask: u32, expected: u32) -> bool {
        for _ in 0..1_000_000u32 {
            if self.read_op_reg(reg) & mask == expected {
                return true;
            }
            spin(100);
        }
        false
    }

    /// Initialize operational registers
    unsafe fn init_operational(&mut self) -> Result<(), UsbError> {
        crate::framebuffer::write_str("xHCI: Setting up DCBAA and ERST...\n");

        // DEBUG: Verify identity mapping assumption
        // Since we're running in raw mode after ExitBootServices with no paging,
        // virtual addresses should equal physical addresses (identity mapping)
        crate::framebuffer::write_str("xHCI: Assuming identity mapping (virt=phys)\n");

        // ============================================================
        // STEP 1: Verify and print all DMA buffer addresses
        // ============================================================

        // Command ring
        let cmd_base = command_ring_base();
        crate::framebuffer::write_str("xHCI: Command ring virt=phys=0x");
        print_hex_u64(cmd_base);
        crate::framebuffer::write_str(" align=");
        if command_ring_aligned() {
            crate::framebuffer::write_str("OK\n");
        } else {
            crate::framebuffer::write_str("BAD!\n");
            return Err(UsbError::XhciInitFailed);
        }

        // Event ring
        let evt_base = event_ring_base();
        crate::framebuffer::write_str("xHCI: Event ring virt=phys=0x");
        print_hex_u64(evt_base);
        crate::framebuffer::write_str(" align=");
        if event_ring_aligned() {
            crate::framebuffer::write_str("OK\n");
        } else {
            crate::framebuffer::write_str("BAD!\n");
            return Err(UsbError::XhciInitFailed);
        }

        // ERST entry
        let erst_addr = erst_base();
        crate::framebuffer::write_str("xHCI: ERST entry virt=phys=0x");
        print_hex_u64(erst_addr);
        crate::framebuffer::write_str(" align=");
        if erst_aligned() {
            crate::framebuffer::write_str("OK\n");
        } else {
            crate::framebuffer::write_str("BAD!\n");
            return Err(UsbError::XhciInitFailed);
        }

        // DCBAA
        let dcbaa_ptr = dcbaa_base();
        crate::framebuffer::write_str("xHCI: DCBAA virt=phys=0x");
        print_hex_u64(dcbaa_ptr);
        crate::framebuffer::write_str(" align=");
        if dcbaa_aligned() {
            crate::framebuffer::write_str("OK\n");
        } else {
            crate::framebuffer::write_str("BAD!\n");
            return Err(UsbError::XhciInitFailed);
        }

        // ============================================================
        // STEP 2: Max slots, scratchpads, DCBAAP
        // ============================================================

        let slots = core::cmp::min(self.max_slots as u32, 8);
        self.write_op_reg(REG_CONFIG, slots);

        if !setup_scratchpad(self.scratchpad_count as usize) {
            crate::framebuffer::write_str("xHCI: ERROR - controller needs more scratchpad pages than reserved\n");
            return Err(UsbError::XhciInitFailed);
        }

        let dcbaa_lo = (dcbaa_ptr & 0xFFFFFFFF) as u32;
        let dcbaa_hi = ((dcbaa_ptr >> 32) & 0xFFFFFFFF) as u32;
        self.write_op_reg(REG_DCBAAP, dcbaa_lo);
        self.write_op_reg(REG_DCBAAP + 4, dcbaa_hi);

        // ============================================================
        // STEP 3: Command ring (CRCR) - must be programmed while halted
        // ============================================================
        // Note: the Command Ring Pointer field always reads back as 0
        // (xHCI 5.4.5), so there is nothing useful to verify by reading it.

        let crcr: u64 = command_ring_base() | (command_ring_cycle() & 1);
        self.write_op_reg(REG_CRCR, (crcr & 0xFFFF_FFFF) as u32);
        self.write_op_reg(REG_CRCR + 4, (crcr >> 32) as u32);

        // ============================================================
        // STEP 4: Event ring for interrupter 0
        // ============================================================

        self.write_runtime_reg(REG_ERSTSZ_0, 1); // 1 segment
        self.write_runtime_reg(REG_ERDP_LO_0, (evt_base & 0xFFFFFFFF) as u32);
        self.write_runtime_reg(REG_ERDP_HI_0, (evt_base >> 32) as u32);
        // Writing ERSTBA (low then high) is what enables the event ring
        self.write_runtime_reg(REG_ERSTBA_LO_0, (erst_addr & 0xFFFFFFFF) as u32);
        self.write_runtime_reg(REG_ERSTBA_HI_0, (erst_addr >> 32) as u32);

        // We poll the event ring; keep the interrupter enabled so IMAN.IP
        // and USBSTS.EINT reflect pending events, but USBCMD.INTE stays off.
        self.write_runtime_reg(REG_IMOD, 0);
        self.write_runtime_reg(REG_IMAN, IMAN_IP | IMAN_IE);

        crate::framebuffer::write_str("xHCI: DCBAA, command ring and event ring programmed\n");

        // ============================================================
        // STEP 5: Run
        // ============================================================

        self.write_op_reg(REG_USBCMD, USBCMD_RUN);
        if !self.wait_op(REG_USBSTS, USBSTS_HCH, 0) {
            crate::framebuffer::write_str("xHCI: ERROR - controller did not start\n");
            return Err(UsbError::XhciInitFailed);
        }
        let usbsts = self.read_op_reg(REG_USBSTS);
        if usbsts & (USBSTS_HSE | USBSTS_HCE) != 0 {
            crate::framebuffer::write_str("xHCI: ERROR - host system/controller error, USBSTS=0x");
            print_hex_u32(usbsts);
            crate::framebuffer::write_str("\n");
            return Err(UsbError::XhciInitFailed);
        }
        crate::framebuffer::write_str("xHCI: Controller running\n");

        // Power every port (needed when the controller supports port power
        // switching; harmless otherwise), then allow ~100 ms+ for devices to
        // connect and debounce after the reset.
        for port in 1..=self.max_ports as usize {
            let portsc = self.read_port_sc(port);
            if portsc & PORTSC_PP == 0 {
                self.write_port_sc(port, (portsc & PORTSC_PRESERVE) | PORTSC_PP);
            }
        }
        spin(150_000_000);

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

    /// Reset a USB2 port and wait until it is enabled.
    /// (USB3 ports enable themselves after link training and need no reset.)
    pub unsafe fn reset_port(&self, port: usize) -> Result<(), UsbError> {
        let portsc = self.read_port_sc(port);
        // Clear stale change bits, then set PR without touching PED
        self.write_port_sc(port, (portsc & PORTSC_PRESERVE) | (portsc & PORTSC_CHANGE_BITS));
        self.write_port_sc(port, (portsc & PORTSC_PRESERVE) | PORTSC_PR);

        let mut done = false;
        for _ in 0..1_000_000u32 {
            let p = self.read_port_sc(port);
            if p & PORTSC_PRC != 0 && p & PORTSC_PR == 0 {
                done = true;
                break;
            }
            spin(100);
        }
        let p = self.read_port_sc(port);
        // Acknowledge PRC (and any other change bits)
        self.write_port_sc(port, (p & PORTSC_PRESERVE) | (p & PORTSC_CHANGE_BITS));

        if !done {
            return Err(UsbError::Timeout);
        }
        if p & PORTSC_PED == 0 {
            return Err(UsbError::DeviceNotFound);
        }
        // USB 2.0 reset recovery time (10 ms)
        spin(5_000_000);
        Ok(())
    }

    /// Take the next event off the event ring, if any, and tell the
    /// controller how far we have read (ERDP with EHB cleared).
    pub unsafe fn poll_events(&self) -> Option<EventTrb> {
        let event = event_ring_dequeue()?;

        let erdp = event_ring_dequeue_ptr();
        self.write_runtime_reg(REG_ERDP_LO_0, ((erdp & 0xFFFF_FFF0) as u32) | ERDP_EHB);
        self.write_runtime_reg(REG_ERDP_HI_0, (erdp >> 32) as u32);

        // Acknowledge interrupt-pending state (both RW1C)
        let iman = self.read_runtime_reg(REG_IMAN);
        if iman & IMAN_IP != 0 {
            self.write_runtime_reg(REG_IMAN, iman | IMAN_IP);
        }
        let sts = self.read_op_reg(REG_USBSTS);
        if sts & (USBSTS_EINT | USBSTS_PCD) != 0 {
            self.write_op_reg(REG_USBSTS, sts & (USBSTS_EINT | USBSTS_PCD));
        }

        Some(event)
    }

    /// Wait for a Command Completion Event (type 33). Other events (e.g.
    /// Port Status Change, type 34) are skipped. Returns (completion code, slot id).
    unsafe fn wait_command_completion(&self) -> Result<(u8, u8), UsbError> {
        for _ in 0..2_000_000u32 {
            if let Some(event) = self.poll_events() {
                let event_type = ((event.control >> 10) & 0x3F) as u8;
                if event_type == 33 {
                    let code = (event.status >> 24) as u8;
                    let slot = ((event.control >> 24) & 0xFF) as u8;
                    return Ok((code, slot));
                }
            } else {
                spin(100);
            }
        }
        Err(UsbError::Timeout)
    }

    /// Wait for a Transfer Event (type 32) for `slot_id`.
    /// Returns (completion code, residual byte count).
    pub unsafe fn wait_transfer_event(&self, slot_id: u8) -> Result<(u8, u32), UsbError> {
        for _ in 0..2_000_000u32 {
            if let Some(event) = self.poll_events() {
                let event_type = ((event.control >> 10) & 0x3F) as u8;
                let slot = ((event.control >> 24) & 0xFF) as u8;
                if event_type == 32 && slot == slot_id {
                    return Ok(((event.status >> 24) as u8, event.status & 0x00FF_FFFF));
                }
            } else {
                spin(100);
            }
        }
        Err(UsbError::Timeout)
    }

    /// Run a command TRB and wait for its completion. Returns the slot id
    /// from the completion event.
    unsafe fn run_command(&mut self, name: &str, trb: NormalTrb) -> Result<u8, UsbError> {
        command_ring_enqueue(&trb)?;
        self.ring_doorbell(0, 0, 0);
        match self.wait_command_completion() {
            Ok((1, slot)) => Ok(slot),
            Ok((code, _)) => {
                crate::framebuffer::write_str("xHCI: ");
                crate::framebuffer::write_str(name);
                crate::framebuffer::write_str(" failed, CC=");
                print_decimal(code as u32);
                crate::framebuffer::write_str(" (");
                crate::framebuffer::write_str(completion_code_name(code));
                crate::framebuffer::write_str(")\n");
                Err(UsbError::XhciInitFailed)
            }
            Err(e) => {
                crate::framebuffer::write_str("xHCI: ");
                crate::framebuffer::write_str(name);
                crate::framebuffer::write_str(" timed out\n");
                Err(e)
            }
        }
    }

    /// Enable Slot command (TRB type 9). Returns the assigned slot id.
    pub unsafe fn issue_enable_slot_command(&mut self) -> Result<u8, UsbError> {
        let trb = NormalTrb { data_ptr: 0, status: 0, control: 9 << 10 };
        let slot = self.run_command("Enable Slot", trb)?;
        if slot == 0 {
            return Err(UsbError::XhciInitFailed);
        }
        Ok(slot)
    }

    /// Disable Slot command (TRB type 10)
    pub unsafe fn issue_disable_slot_command(&mut self, slot_id: u8) -> Result<(), UsbError> {
        let trb = NormalTrb { data_ptr: 0, status: 0, control: ((slot_id as u32) << 24) | (10 << 10) };
        self.run_command("Disable Slot", trb).map(|_| ())
    }

    /// Address Device command (TRB type 11). With BSR=false the controller
    /// also sends SET_ADDRESS to the device.
    pub unsafe fn issue_address_device_command(
        &mut self,
        slot_id: u8,
        input_context_ptr: u64,
        bsr: bool,
    ) -> Result<(), UsbError> {
        let bsr_bit: u32 = if bsr { 1 << 9 } else { 0 };
        let trb = NormalTrb {
            data_ptr: input_context_ptr,
            status: 0,
            control: ((slot_id as u32) << 24) | (11 << 10) | bsr_bit,
        };
        self.run_command("Address Device", trb).map(|_| ())
    }

    /// Configure Endpoint command (TRB type 12)
    pub unsafe fn issue_configure_endpoint_command(
        &mut self,
        slot_id: u8,
        input_context_ptr: u64,
    ) -> Result<(), UsbError> {
        let trb = NormalTrb {
            data_ptr: input_context_ptr,
            status: 0,
            control: ((slot_id as u32) << 24) | (12 << 10),
        };
        self.run_command("Configure Endpoint", trb).map(|_| ())
    }

    /// Evaluate Context command (TRB type 13), used to update EP0 max packet size
    pub unsafe fn issue_evaluate_context_command(
        &mut self,
        slot_id: u8,
        input_context_ptr: u64,
    ) -> Result<(), UsbError> {
        let trb = NormalTrb {
            data_ptr: input_context_ptr,
            status: 0,
            control: ((slot_id as u32) << 24) | (13 << 10),
        };
        self.run_command("Evaluate Context", trb).map(|_| ())
    }

    /// Control transfer with an IN data stage (e.g. GET_DESCRIPTOR).
    /// Returns the number of bytes received.
    pub unsafe fn issue_control_get(
        &mut self,
        slot_id: u8,
        setup: &crate::usb::device::UsbSetupPacket,
        data_ptr: u64,
        data_length: u16,
    ) -> Result<u32, UsbError> {
        // Setup Stage: IDT, TRT=3 (IN data stage), transfer length is always 8
        ep0_ring_enqueue(&NormalTrb {
            data_ptr: setup_packet_u64(setup),
            status: 8,
            control: (2 << 10) | (3 << 16) | (1 << 6),
        })?;
        // Data Stage: DIR=IN; ISP so a short packet ends the TD with an event
        ep0_ring_enqueue(&NormalTrb {
            data_ptr,
            status: (data_length as u32) & 0x1_FFFF,
            control: (3 << 10) | (1 << 16) | (1 << 2),
        })?;
        // Status Stage: DIR=OUT, IOC
        ep0_ring_enqueue(&NormalTrb {
            data_ptr: 0,
            status: 0,
            control: (4 << 10) | (1 << 5),
        })?;
        self.ring_doorbell(slot_id, 1, 0);
        self.finish_control_transfer(slot_id, data_length as u32)
    }

    /// Control transfer without a data stage (SET_CONFIGURATION, SET_PROTOCOL, ...)
    pub unsafe fn issue_control_set(
        &mut self,
        slot_id: u8,
        setup: &crate::usb::device::UsbSetupPacket,
    ) -> Result<(), UsbError> {
        // Setup Stage: IDT, TRT=0 (no data stage)
        ep0_ring_enqueue(&NormalTrb {
            data_ptr: setup_packet_u64(setup),
            status: 8,
            control: (2 << 10) | (1 << 6),
        })?;
        // Status Stage: DIR=IN, IOC
        ep0_ring_enqueue(&NormalTrb {
            data_ptr: 0,
            status: 0,
            control: (4 << 10) | (1 << 16) | (1 << 5),
        })?;
        self.ring_doorbell(slot_id, 1, 0);
        self.finish_control_transfer(slot_id, 0).map(|_| ())
    }

    /// Wait for the end of a control transfer. A short packet on the data
    /// stage produces an extra event (CC=13) before the status stage event.
    unsafe fn finish_control_transfer(&self, slot_id: u8, requested: u32) -> Result<u32, UsbError> {
        let mut received = requested;
        loop {
            let (code, residual) = self.wait_transfer_event(slot_id)?;
            match code {
                13 => {
                    // Short packet on data stage; status stage event follows
                    received = requested.saturating_sub(residual);
                }
                1 => return Ok(received),
                _ => {
                    crate::framebuffer::write_str("xHCI: control transfer failed, CC=");
                    print_decimal(code as u32);
                    crate::framebuffer::write_str(" (");
                    crate::framebuffer::write_str(completion_code_name(code));
                    crate::framebuffer::write_str(")\n");
                    return Err(UsbError::XhciInitFailed);
                }
            }
        }
    }
}

/// Pack a setup packet into the 8-byte immediate data of a Setup TRB
fn setup_packet_u64(setup: &crate::usb::device::UsbSetupPacket) -> u64 {
    setup.bm_request_type as u64
        | ((setup.b_request as u64) << 8)
        | ((setup.w_value as u64) << 16)
        | ((setup.w_index as u64) << 32)
        | ((setup.w_length as u64) << 48)
}

/// Human-readable completion code
pub fn completion_code_name(code: u8) -> &'static str {
    match code {
        1 => "Success",
        2 => "Data Buffer Error",
        3 => "Babble",
        4 => "USB Transaction Error",
        5 => "TRB Error",
        6 => "Stall",
        7 => "Resource Error",
        8 => "Bandwidth Error",
        9 => "No Slots Available",
        11 => "Slot Not Enabled",
        12 => "Endpoint Not Enabled",
        13 => "Short Packet",
        17 => "Parameter Error",
        19 => "Context State Error",
        _ => "Other",
    }
}

/// Busy-wait for roughly `n` iterations
fn spin(n: u32) {
    for _ in 0..n {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

/// Walk the extended capability list and, if the BIOS owns the controller
/// (USB Legacy Support capability), request ownership and turn off the
/// BIOS's SMI handlers. Without this the firmware may keep driving the
/// controller (legacy keyboard emulation) and fight with our driver.
unsafe fn bios_handoff(mmio_base: *mut u8, hccparams1: u32) {
    let mut offset = ((hccparams1 >> 16) & 0xFFFF) as usize * 4;
    let mut guard = 0;
    while offset != 0 && guard < 64 {
        guard += 1;
        let cap_ptr = mmio_base.add(offset).cast::<u32>();
        let cap = cap_ptr.read_volatile();
        if cap & 0xFF == XECP_ID_LEGACY {
            crate::framebuffer::write_str("xHCI: BIOS handoff... ");
            if cap & USBLEGSUP_BIOS_OWNED != 0 {
                cap_ptr.write_volatile(cap | USBLEGSUP_OS_OWNED);
                let mut released = false;
                for _ in 0..1_000_000u32 {
                    if cap_ptr.read_volatile() & USBLEGSUP_BIOS_OWNED == 0 {
                        released = true;
                        break;
                    }
                    spin(200);
                }
                if released {
                    crate::framebuffer::write_str("released by BIOS\n");
                } else {
                    // BIOS did not respond: take it anyway
                    let v = cap_ptr.read_volatile();
                    cap_ptr.write_volatile((v & !USBLEGSUP_BIOS_OWNED) | USBLEGSUP_OS_OWNED);
                    crate::framebuffer::write_str("BIOS timeout, forced\n");
                }
            } else {
                cap_ptr.write_volatile(cap | USBLEGSUP_OS_OWNED);
                crate::framebuffer::write_str("not BIOS-owned\n");
            }
            // USBLEGCTLSTS: disable all SMI enables, clear SMI status (RW1C)
            let ctl = cap_ptr.add(1);
            let v = ctl.read_volatile();
            ctl.write_volatile((v & 0xE000_0000) & !0x0000_E011);
            return;
        }
        let next = ((cap >> 8) & 0xFF) as usize;
        if next == 0 {
            break;
        }
        offset += next * 4;
    }
}

/// Helper: print a u32 in decimal
unsafe fn print_decimal(mut value: u32) {
    let mut digits = [0u8; 10];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (value % 10) as u8;
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for i in (0..len).rev() {
        crate::framebuffer::write_str(core::str::from_utf8(&[digits[i]]).unwrap());
    }
}

/// Helper: Print 64-bit hex value
unsafe fn print_hex_u64(value: u64) {
    for i in (0..64).step_by(4).rev() {
        let nibble = (value >> i) & 0xF;
        let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
        crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
    }
}

/// Helper: Print 32-bit hex value
unsafe fn print_hex_u32(value: u32) {
    for i in (0..32).step_by(4).rev() {
        let nibble = (value >> i) & 0xF;
        let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
        crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
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
