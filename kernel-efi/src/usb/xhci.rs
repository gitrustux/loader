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

/// Runtime register offsets
const REG_MFINDEX: usize = 0x000;
const REG_ERSTSZ: usize = 0x008; // Event Ring Segment Table Size
const REG_ERSTBA_LO: usize = 0x010; // Event Ring Segment Table Base Address Low
const REG_ERSTBA_HI: usize = 0x014; // Event Ring Segment Table Base Address High
const REG_ERDP_LO: usize = 0x020; // Event Ring Dequeue Pointer
const REG_ERDP_HI: usize = 0x024;
const REG_IMAN: usize = 0x000; // Interrupter Management (for interrupter 0)
const REG_IMOD: usize = 0x004; // Interrupter Moderation
const REG_ERSTSZ_0: usize = 0x008; // ERST Size for interrupter 0
const REG_ERSTBA_LO_0: usize = 0x010; // ERST Base Low for interrupter 0
const REG_ERSTBA_HI_0: usize = 0x014; // ERST Base High for interrupter 0
const REG_ERDP_LO_0: usize = 0x020; // ERDP Low for interrupter 0
const REG_ERDP_HI_0: usize = 0x024; // ERDP High for interrupter 0

/// Interrupter Management bits
const IMAN_IP: u32 = 0x0000_0001; // Interrupt Pending
const IMAN_IE: u32 = 0x0000_0002; // Interrupt Enable

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

/// Port status register bits (xHCI spec section 5.4.8)
pub const PORTSC_CCS: u32 = 0x0000_0001; // Bit 0: Current Connect Status
const PORTSC_CSC_FLAG: u32 = 0x0000_0002; // Bit 1: Connect Status Change (renamed to avoid conflict)
pub const PORTSC_PED: u32 = 0x0000_0004; // Bit 2: Port Enabled/Disabled (FIXED - was 0x0002)
const PORTSC_PEDC: u32 = 0x0000_0008; // Bit 3: Port Enabled/Disabled Change (FIXED - was 0x0010)
const PORTSC_OCA: u32 = 0x0000_0010; // Bit 4: Over-current Active
pub const PORTSC_PR: u32 = 0x0000_0020; // Bit 5: Port Reset (FIXED - was 0x0010)
const PORTSC_PLS_MASK: u32 = 0x0000_03E0; // Bits 9-5: Port Link State (fixed range)
const PORTSC_PLS_U0: u32 = 0x0000_0000; // U0 state
const PORTSC_PP: u32 = 0x0000_0200; // Bit 9: Port Power
const PORTSC_PR_MASK: u32 = 0x0000_0020; // Bit 5: Port Reset mask for clearing
pub const PORTSC_PORT_SPEED_MASK: u32 = 0x0000_F000; // Bits 15-12: Port Speed
pub const PORTSC_PORT_SPEED_SHIFT: u32 = 12;

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

        // Initialize ERST with event ring address
        unsafe {
            erst_init();
        }

        let mut controller = Self {
            mmio_base: info.mmio_base,
            cap_length,
            runtime_offset,
            doorbell_offset,
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

    /// Read port status register
    pub unsafe fn read_port_sc(&self, port: usize) -> u32 {
        self.read_op_reg(REG_PORTSC_BASE + (port * 0x10))
    }

    /// Write port status register
    pub unsafe fn write_port_sc(&self, port: usize, value: u32) {
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
        // STEP 2: Program DCBAAP (Device Context Base Address Array Pointer)
        // ============================================================

        let dcbaa_lo = (dcbaa_ptr & 0xFFFFFFFF) as u32;
        let dcbaa_hi = ((dcbaa_ptr >> 32) & 0xFFFFFFFF) as u32;
        self.write_op_reg(REG_DCBAAP, dcbaa_lo);
        self.write_op_reg(REG_DCBAAP + 4, dcbaa_hi);

        crate::framebuffer::write_str("xHCI: DCBAAP programmed\n");

        // ============================================================
        // STEP 3: Program ERST (Event Ring Segment Table)
        // ============================================================

        // Set ERST size (number of segments)
        self.write_runtime_reg(REG_ERSTSZ_0, 1); // 1 segment

        // Set ERST base address (pointer to ERST entries)
        let erst_lo = (erst_addr & 0xFFFFFFFF) as u32;
        let erst_hi = ((erst_addr >> 32) & 0xFFFFFFFF) as u32;
        self.write_runtime_reg(REG_ERSTBA_LO_0, erst_lo);
        self.write_runtime_reg(REG_ERSTBA_HI_0, erst_hi);

        // Initialize ERDP to point to event ring start
        let erdp_lo = (evt_base & 0xFFFFFFFF) as u32;
        let erdp_hi = ((evt_base >> 32) & 0xFFFFFFFF) as u32;
        self.write_runtime_reg(REG_ERDP_LO_0, erdp_lo);
        self.write_runtime_reg(REG_ERDP_HI_0, erdp_hi);

        // Enable interrupter 0
        let iman = self.read_runtime_reg(REG_IMAN);
        self.write_runtime_reg(REG_IMAN, iman | IMAN_IE); // Enable interrupter

        // Set interrupt moderation to avoid overwhelming the system
        self.write_runtime_reg(REG_IMOD, 500); // 500 microseconds

        crate::framebuffer::write_str("xHCI: ERST configured, interrupter enabled\n");

        // ============================================================
        // STEP 4: Set max device slots and start controller
        // ============================================================

        self.write_op_reg(REG_CONFIG, 8); // Max slots = 8

        // Start controller FIRST (before programming CRCR)
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
            crate::framebuffer::write_str("xHCI: Controller Not Ready timeout\n");
            return Err(UsbError::XhciInitFailed);
        }

        // Verify controller is actually running (HCH == 0 means running)
        let usbsts = self.read_op_reg(REG_USBSTS);
        if usbsts & USBSTS_HCH != 0 {
            crate::framebuffer::write_str("xHCI: ERROR - Controller Halted!\n");
            return Err(UsbError::XhciInitFailed);
        }
        crate::framebuffer::write_str("xHCI: Controller running OK\n");

        // ============================================================
        // STEP 5: Initialize CRCR ONCE during controller setup
        // ============================================================
        // CRITICAL: CRCR must point to command ring BASE, not enqueue position
        // Format: [63:6] Command Ring Base (64-byte aligned)
        //         [5:1] Reserved (must be 0)
        //         [0] RCS (Ring Cycle State)
        //
        // Per xHCI spec 5.4.5: CRCR is initialized ONCE, then never modified
        // except for RCS bit toggling when ring wraps.

        // FRESH read of command ring base - use function call to ensure no caching
        let cmd_ring_base: u64 = command_ring_base();
        let cmd_cycle: u64 = command_ring_cycle();

        crate::framebuffer::write_str("xHCI: CRCR calculation:\n");
        crate::framebuffer::write_str("xHCI:   cmd_ring_base=0x");
        print_hex_u64(cmd_ring_base);
        crate::framebuffer::write_str("\n");
        crate::framebuffer::write_str("xHCI:   cmd_cycle=");
        crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + cmd_cycle as u8]).unwrap());
        crate::framebuffer::write_str("\n");

        // Verify alignment one more time before programming
        if cmd_ring_base & 0x3F != 0 {
            crate::framebuffer::write_str("xHCI: ERROR - Command ring not 64-byte aligned!\n");
            return Err(UsbError::XhciInitFailed);
        }

        let crcr: u64 = cmd_ring_base | cmd_cycle;

        crate::framebuffer::write_str("xHCI: CRCR calculated=0x");
        print_hex_u64(crcr);
        crate::framebuffer::write_str("\n");

        // Check CRR (Command Ring Running) bit before writing
        let crcr_before = self.read_op_reg(REG_CRCR);
        crate::framebuffer::write_str("xHCI: CRCR before=0x");
        print_hex_u32(crcr_before);
        crate::framebuffer::write_str("\n");
        if crcr_before & (1 << 3) != 0 {
            crate::framebuffer::write_str("xHCI: WARNING - CRR bit set, ring running!\n");
        }

        // Write CRCR as two 32-bit registers
        let crcr_lo: u32 = (crcr & 0xFFFFFFFF) as u32;
        let crcr_hi: u32 = ((crcr >> 32) & 0xFFFFFFFF) as u32;

        crate::framebuffer::write_str("xHCI: CRCR_LO=0x");
        print_hex_u32(crcr_lo);
        crate::framebuffer::write_str("\n");
        crate::framebuffer::write_str("xHCI: CRCR_HI=0x");
        print_hex_u32(crcr_hi);
        crate::framebuffer::write_str("\n");

        // Calculate actual MMIO addresses for debugging
        let mmio_base = self.mmio_base as *mut u8;
        let op_base = mmio_base.add(self.cap_length as usize);
        let crcr_lo_addr = op_base.add(REG_CRCR) as u64;
        let crcr_hi_addr = op_base.add(REG_CRCR + 4) as u64;

        crate::framebuffer::write_str("xHCI: MMIO base=0x");
        print_hex_u64(self.mmio_base);
        crate::framebuffer::write_str("\n");
        crate::framebuffer::write_str("xHCI: Writing CRCR_LO to addr=0x");
        print_hex_u64(crcr_lo_addr);
        crate::framebuffer::write_str(" val=0x");
        print_hex_u32(crcr_lo);
        crate::framebuffer::write_str("\n");
        crate::framebuffer::write_str("xHCI: Writing CRCR_HI to addr=0x");
        print_hex_u64(crcr_hi_addr);
        crate::framebuffer::write_str(" val=0x");
        print_hex_u32(crcr_hi);
        crate::framebuffer::write_str("\n");

        // Write HI first (some xHCI controllers require this order)
        self.write_op_reg(REG_CRCR + 4, crcr_hi);
        // Small delay between writes
        for _ in 0..100 {
            core::arch::asm!("nop", options(nomem, nostack));
        }
        self.write_op_reg(REG_CRCR, crcr_lo);

        // Verify CRCR was written correctly
        let crcr_lo_read = self.read_op_reg(REG_CRCR);
        let crcr_hi_read = self.read_op_reg(REG_CRCR + 4);
        let crcr_read: u64 = (crcr_hi_read as u64) << 32 | (crcr_lo_read as u64);

        crate::framebuffer::write_str("xHCI: CRCR_LO readback=0x");
        print_hex_u32(crcr_lo_read);
        crate::framebuffer::write_str("\n");
        crate::framebuffer::write_str("xHCI: CRCR_HI readback=0x");
        print_hex_u32(crcr_hi_read);
        crate::framebuffer::write_str("\n");
        crate::framebuffer::write_str("xHCI: CRCR readback=0x");
        print_hex_u64(crcr_read);
        if crcr_read == crcr {
            crate::framebuffer::write_str(" OK\n");
        } else {
            crate::framebuffer::write_str(" MISMATCH!\n");
            // Print expected vs actual
            crate::framebuffer::write_str("xHCI: Expected=0x");
            print_hex_u64(crcr);
            crate::framebuffer::write_str(" Got=0x");
            print_hex_u64(crcr_read);
            crate::framebuffer::write_str("\n");
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
        // Update ERDP to clear events (use interrupter 0 registers)
        let erdp = event_ring_dequeue_ptr();
        self.write_runtime_reg(REG_ERDP_LO_0, (erdp & 0xFFFF_FFFF) as u32);
        self.write_runtime_reg(REG_ERDP_HI_0, ((erdp >> 32) & 0xFFFF_FFFF) as u32);

        // Check for pending events in IMAN
        let iman = self.read_runtime_reg(REG_IMAN);
        if iman & IMAN_IP != 0 {
            // Event pending - clear it by writing 1 to IP
            self.write_runtime_reg(REG_IMAN, IMAN_IP);
        }

        // Try to dequeue an event
        event_ring_dequeue()
    }

    /// Issue Enable Slot command and wait for completion
    /// Returns the assigned slot ID on success
    ///
    /// IMPORTANT: CRCR is already set during controller initialization.
    /// Commands are submitted by:
    /// 1. Enqueue TRB to command ring
    /// 2. Ring doorbell 0
    /// 3. Poll event ring for completion
    pub unsafe fn issue_enable_slot_command(&mut self) -> Result<u8, UsbError> {
        crate::framebuffer::write_str("xHCI: Enable Slot command...\n");

        // Verify controller is running before issuing commands
        let usbsts = self.read_op_reg(REG_USBSTS);
        if usbsts & USBSTS_HCH != 0 {
            crate::framebuffer::write_str("xHCI: ERROR - Controller halted, cannot issue command\n");
            return Err(UsbError::XhciInitFailed);
        }

        // Show command ring state before enqueue
        let enqueue_idx = command_ring_enqueue_idx();
        let cycle = command_ring_cycle_value();

        crate::framebuffer::write_str("xHCI: CMD idx=");
        crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + enqueue_idx as u8]).unwrap());
        crate::framebuffer::write_str(" cycle=");
        crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + cycle as u8]).unwrap());
        crate::framebuffer::write_str("\n");

        // Create Enable Slot TRB (TRB Type 9)
        // No parameters needed for Enable Slot
        let trb = NormalTrb {
            data_ptr: 0,
            status: 0,
            control: (9 << 10) | (1 << 5), // TRB Type=9, IOC (no IMMED for Enable Slot)
        };

        // Debug: Print TRB contents
        crate::framebuffer::write_str("xHCI: TRB Type=9 (Enable Slot)\n");

        // Enqueue to command ring
        command_ring_enqueue(&trb)?;

        // Ring command doorbell (doorbell 0) to notify controller
        // NOTE: CRCR was already set during init, controller tracks dequeue internally
        crate::framebuffer::write_str("xHCI: Ringing doorbell 0...\n");
        self.ring_doorbell(0, 0, 0);

        // Wait for command completion event
        let mut events_seen = 0;
        let mut timeout = 100000;
        while timeout > 0 {
            if let Some(event) = self.poll_events() {
                events_seen += 1;

                // Dump raw event TRB for debugging
                crate::framebuffer::write_str("xHCI: Event ");
                print_hex_u64(event.trb_ptr);
                crate::framebuffer::write_str(" ");
                print_hex_u32(event.status);
                crate::framebuffer::write_str(" ");
                print_hex_u32(event.control);
                crate::framebuffer::write_str("\n");

                // Check if this is a command completion event (Type 33)
                let event_type = ((event.control >> 10) & 0x3F) as u8;
                crate::framebuffer::write_str("xHCI: Event Type=");
                crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + event_type as u8]).unwrap());

                // Extract completion code
                let code = (event.status >> 24) as u8;
                crate::framebuffer::write_str(" CC=");
                crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + code as u8]).unwrap());

                if event_type == 33 {
                    // Command completion event
                    if code == 1 {
                        // Success - extract slot ID from control field
                        let slot_id = ((event.control >> 24) & 0xFF) as u8;
                        crate::framebuffer::write_str(" Slot=");
                        if slot_id < 10 {
                            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + slot_id]).unwrap());
                        } else {
                            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + (slot_id / 10)]).unwrap());
                            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + (slot_id % 10)]).unwrap());
                        }
                        crate::framebuffer::write_str("\n");
                        return Ok(slot_id);
                    } else {
                        // Command failed
                        crate::framebuffer::write_str(" ERROR\n");
                        return Err(UsbError::XhciInitFailed);
                    }
                } else {
                    // Not a command completion event (unexpected)
                    crate::framebuffer::write_str(" (not command completion)\n");
                }
            }
            timeout -= 1;
            for _ in 0..100 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }

        // Timeout - no events received
        crate::framebuffer::write_str("xHCI: Timeout (");
        // Print events seen count
        if events_seen < 10 {
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + events_seen as u8]).unwrap());
        }
        crate::framebuffer::write_str(" events)\n");

        // Try to dump event ring state even on timeout
        crate::framebuffer::write_str("xHCI: Event ring state:\n");
        let (evt_dequeue, evt_cycle) = event_ring_state();
        crate::framebuffer::write_str("xHCI:  dequeue=");
        crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + evt_dequeue as u8]).unwrap());
        crate::framebuffer::write_str(" cycle=");
        crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + evt_cycle as u8]).unwrap());
        crate::framebuffer::write_str("\n");

        // Dump first event TRB regardless of cycle state
        let first_trb = event_ring_first_trb();
        crate::framebuffer::write_str("xHCI:  First TRB: ");
        print_hex_u64(first_trb.trb_ptr);
        crate::framebuffer::write_str(" ");
        print_hex_u32(first_trb.status);
        crate::framebuffer::write_str(" ");
        print_hex_u32(first_trb.control);
        crate::framebuffer::write_str("\n");

        Err(UsbError::Timeout)
    }

    /// Issue control transfer GET (read data from device)
    /// Used for GET_DESCRIPTOR and other control-IN transfers
    pub unsafe fn issue_control_get(
        &mut self,
        slot_id: u8,
        setup: &crate::usb::device::UsbSetupPacket,
        data_ptr: u64,
        data_length: u16,
    ) -> Result<(), UsbError> {
        crate::framebuffer::write_str("xHCI: Control GET transfer...\n");

        // Step 1: Setup Stage TRB
        let setup_trb = NormalTrb {
            data_ptr: setup.bm_request_type as u64 |
                      ((setup.b_request as u64) << 8) |
                      ((setup.w_value as u64) << 16) |
                      ((setup.w_index as u64) << 32) |
                      ((setup.w_length as u64) << 48),
            status: (data_length as u32) & 0x00FFFFFF,
            control: (2 << 10) | (1 << 16) | (1 << 5) | (1 << 6), // TRB Type=2, Direction=IN, IOC, IMMED
        };

        self.transfer_ring.enqueue(&setup_trb)?;

        // Step 2: Data Stage TRB (Normal TRB with data buffer)
        let data_trb = NormalTrb {
            data_ptr,
            status: (data_length as u32) & 0x00FFFFFF,
            control: (1 << 10) | (1 << 16) | (1 << 5), // TRB Type=1, Direction=IN, IOC
        };

        self.transfer_ring.enqueue(&data_trb)?;

        // Step 3: Status Stage TRB (Direction=OUT for control-IN)
        let status_trb = NormalTrb {
            data_ptr: 0,
            status: 0,
            control: (4 << 10) | (1 << 5), // TRB Type=4, IOC
        };

        self.transfer_ring.enqueue(&status_trb)?;

        // Ring doorbell for Default Control Endpoint (EP0)
        // DCI for control endpoint is always 1 (EP0 IN/OUT)
        self.ring_doorbell(slot_id, 1, 0);

        // Wait for completion events
        let mut events_received = 0;
        let mut timeout = 100000;
        while timeout > 0 && events_received < 3 {
            if let Some(event) = self.poll_events() {
                events_received += 1;
                // Extract completion code
                let code = (event.status >> 24) as u8;
                if code != 1 {
                    crate::framebuffer::write_str("xHCI: Control transfer event CC=");
                    let code_hi = (code >> 4) & 0xF;
                    let code_lo = code & 0xF;
                    let c1 = if code_hi < 10 { b'0' + code_hi } else { b'A' + (code_hi - 10) };
                    let c2 = if code_lo < 10 { b'0' + code_lo } else { b'A' + (code_lo - 10) };
                    crate::framebuffer::write_str(core::str::from_utf8(&[c1, c2]).unwrap());
                    crate::framebuffer::write_str("\n");
                }
            }
            timeout -= 1;
            for _ in 0..100 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }

        if timeout == 0 {
            crate::framebuffer::write_str("xHCI: Control transfer timeout\n");
            return Err(UsbError::Timeout);
        }

        crate::framebuffer::write_str("xHCI: Control GET OK\n");
        Ok(())
    }

    /// Issue control transfer SET (write data to device, or no data)
    /// Used for SET_ADDRESS, SET_CONFIGURATION, SET_PROTOCOL, etc.
    pub unsafe fn issue_control_set(
        &mut self,
        slot_id: u8,
        setup: &crate::usb::device::UsbSetupPacket,
    ) -> Result<(), UsbError> {
        crate::framebuffer::write_str("xHCI: Control SET transfer...\n");

        // Step 1: Setup Stage TRB
        let setup_trb = NormalTrb {
            data_ptr: setup.bm_request_type as u64 |
                      ((setup.b_request as u64) << 8) |
                      ((setup.w_value as u64) << 16) |
                      ((setup.w_index as u64) << 32) |
                      ((setup.w_length as u64) << 48),
            status: (setup.w_length as u32) & 0x00FFFFFF,
            control: (2 << 10) | (1 << 5) | (1 << 6), // TRB Type=2, Direction=OUT, IOC, IMMED
        };

        self.transfer_ring.enqueue(&setup_trb)?;

        // Step 2: Status Stage TRB (Direction=IN for control-OUT)
        let status_trb = NormalTrb {
            data_ptr: 0,
            status: 0,
            control: (4 << 10) | (1 << 16) | (1 << 5), // TRB Type=4, Direction=IN, IOC
        };

        self.transfer_ring.enqueue(&status_trb)?;

        // Ring doorbell for Default Control Endpoint (EP0)
        self.ring_doorbell(slot_id, 1, 0);

        // Wait for completion events
        let mut events_received = 0;
        let mut timeout = 100000;
        while timeout > 0 && events_received < 2 {
            if let Some(event) = self.poll_events() {
                events_received += 1;
                // Extract completion code
                let code = (event.status >> 24) as u8;
                if code != 1 {
                    crate::framebuffer::write_str("xHCI: Control SET event CC=");
                    let code_hi = (code >> 4) & 0xF;
                    let code_lo = code & 0xF;
                    let c1 = if code_hi < 10 { b'0' + code_hi } else { b'A' + (code_hi - 10) };
                    let c2 = if code_lo < 10 { b'0' + code_lo } else { b'A' + (code_lo - 10) };
                    crate::framebuffer::write_str(core::str::from_utf8(&[c1, c2]).unwrap());
                    crate::framebuffer::write_str("\n");
                }
            }
            timeout -= 1;
            for _ in 0..100 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }

        if timeout == 0 {
            crate::framebuffer::write_str("xHCI: Control SET timeout\n");
            return Err(UsbError::Timeout);
        }

        crate::framebuffer::write_str("xHCI: Control SET OK\n");
        Ok(())
    }
}

/// Helper: Print 64-bit hex value
unsafe fn print_hex_u64(value: u64) {
    for i in (0..64).step_by(8).rev() {
        let nibble = (value >> i) & 0xF;
        let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
        crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
    }
}

/// Helper: Print 32-bit hex value
unsafe fn print_hex_u32(value: u32) {
    for i in (0..32).step_by(8).rev() {
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
