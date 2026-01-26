// Copyright 2025 The Rustux Authors
//
// EHCI Controller - USB 2.0 Host Controller Interface
//
// This module implements EHCI controller driver for USB 2.0 support.
// EHCI is used on many laptops as the primary USB controller.

use crate::usb::{EhciInfo, UsbError};

/// EHCI Capability Register Length (CAPLENGTH)
const REG_CAPLENGTH: u8 = 0x00;

/// EHCI Operational Registers
const REG_USBCMD: usize = 0x000;
const REG_USBSTS: usize = 0x004;
const REG_USBINTR: usize = 0x008;
const REG_FRINDEX: usize = 0x00C;
const REG_CTRLDSSEGMENT: usize = 0x010;
const REG_PERIODICLISTBASE: usize = 0x014;
const REG_ASYNCLISTADDR: usize = 0x018;
const REG_CONFIGFLAG: usize = 0x040;

/// Port register set base
const REG_PORTSC_BASE: usize = 0x064;

/// USB command register bits
const USBCMD_RUN: u32 = 0x0000_0001;
const USBCMD_HCRST: u32 = 0x0000_0002;
const USBCMD_FLS: u32 = 0x0000_0000; // Frame List Size (default 1024)

/// USB status register bits
const USBSTS_HCH: u32 = 0x0000_1000; // HCHalted
const USBSTS_HSE: u32 = 0x0000_0010; // Host System Error
const USBSTS_INT: u32 = 0x0000_0004; // USB Interrupt

/// Port status register bits
pub const PORTSC_CCS: u32 = 0x0000_0001; // Current Connect Status
const PORTSC_CSC: u32 = 0x0000_0002; // Connect Status Change
const PORTSC_PED: u32 = 0x0000_0004; // Port Enabled/Disabled
const PORTSC_PEC: u32 = 0x0000_0008; // Port Enable/Disable Change
const PORTSC_PR: u32 = 0x0000_0010; // Port Reset
const PORTSC_SUSP: u32 = 0x0000_0080; // Suspend
const PORTSC_OWNER: u32 = 0x0000_0200; // Port Owner (0=EHCI, 1=companion)

/// EHCI controller state
pub struct EhciController {
    mmio_base: u64,
    cap_length: u8,
}

impl EhciController {
    /// Initialize EHCI controller from MMIO base
    pub unsafe fn new(info: EhciInfo) -> Result<Self, UsbError> {
        let mmio_base = info.mmio_base as *mut u8;

        crate::framebuffer::write_str("EHCI: Reading CAPLENGTH...\n");

        // Read CAPLENGTH to get capability register length
        let cap_length = mmio_base.add(REG_CAPLENGTH as usize).read_volatile() as u8;

        // Verify MMIO is working
        if cap_length == 0xFF {
            crate::framebuffer::write_str("ERROR: CAPLENGTH = 0xFF - MMIO not mapped!\n");
            return Err(UsbError::EhciInitFailed);
        }

        crate::framebuffer::write_str("EHCI: CAPLENGTH = ");
        if cap_length < 10 {
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + cap_length]).unwrap());
        } else {
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + (cap_length / 10)]).unwrap());
            crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + (cap_length % 10)]).unwrap());
        }
        crate::framebuffer::write_str("\n");

        // Read HCIVERSION (offset 0x02 in capability registers)
        let hciversion = mmio_base.add(0x02).cast::<u16>().read_volatile();

        crate::framebuffer::write_str("EHCI: HCIVERSION = 0x");
        let mut version = hciversion;
        for _ in 0..4 {
            let nibble = (version & 0xF000) >> 12;
            let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
            crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
            version <<= 4;
        }
        crate::framebuffer::write_str("\n");

        if hciversion < 0x0100 {
            crate::framebuffer::write_str("ERROR: Unsupported EHCI version (< 1.0)\n");
            return Err(UsbError::EhciInitFailed);
        }

        crate::framebuffer::write_str("EHCI: Resetting controller...\n");

        let mut controller = Self {
            mmio_base: info.mmio_base,
            cap_length,
        };

        // Reset and initialize controller
        controller.reset()?;
        controller.init_operational()?;

        crate::framebuffer::write_str("EHCI: Controller ready\n");

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

    /// Read port status register
    pub unsafe fn read_port_sc(&self, port: usize) -> u32 {
        self.read_op_reg(REG_PORTSC_BASE + (port * 4))
    }

    /// Write port status register
    unsafe fn write_port_sc(&self, port: usize, value: u32) {
        self.write_op_reg(REG_PORTSC_BASE + (port * 4), value);
    }

    /// Reset EHCI controller
    unsafe fn reset(&mut self) -> Result<(), UsbError> {
        // Check if controller is halted
        let status = self.read_op_reg(REG_USBSTS);
        if status & USBSTS_HCH == 0 {
            // Controller running - issue reset
            self.write_op_reg(REG_USBCMD, USBCMD_HCRST);

            // Wait for reset to complete
            let mut timeout = 100000;
            while timeout > 0 {
                let cmd = self.read_op_reg(REG_USBCMD);
                if cmd & USBCMD_HCRST == 0 {
                    return Ok(());
                }

                timeout -= 1;
                for _ in 0..100 {
                    core::arch::asm!("nop", options(nomem, nostack));
                }
            }

            return Err(UsbError::EhciInitFailed);
        }

        Ok(())
    }

    /// Initialize operational registers
    unsafe fn init_operational(&mut self) -> Result<(), UsbError> {
        // Set ConfigFlag to take ownership of ports
        self.write_op_reg(REG_CONFIGFLAG, 1);

        // Start controller
        self.write_op_reg(REG_USBCMD, USBCMD_RUN | USBCMD_FLS);

        // Wait for controller to be ready
        let mut timeout = 100000;
        while timeout > 0 {
            let status = self.read_op_reg(REG_USBSTS);
            if status & USBSTS_HCH == 0 {
                break;
            }
            timeout -= 1;
            for _ in 0..100 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }

        if timeout == 0 {
            return Err(UsbError::EhciInitFailed);
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
            return Err(UsbError::EhciInitFailed);
        }

        // Check if port is enabled
        portsc = self.read_port_sc(port);
        if portsc & PORTSC_PED == 0 {
            return Err(UsbError::DeviceNotFound);
        }

        Ok(())
    }
}

/// Global EHCI controller instance
static mut EHCI_CONTROLLER: Option<EhciController> = None;

/// Initialize EHCI controller
pub unsafe fn init() -> Result<(), UsbError> {
    let info = super::pci::scan_for_ehci()?;
    let controller = EhciController::new(info)?;
    EHCI_CONTROLLER = Some(controller);
    Ok(())
}

/// Get EHCI controller reference
pub unsafe fn controller() -> Option<&'static EhciController> {
    EHCI_CONTROLLER.as_ref()
}

/// Get EHCI controller mutable reference
pub unsafe fn controller_mut() -> Option<&'static mut EhciController> {
    EHCI_CONTROLLER.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(USBCMD_RUN, 0x0000_0001);
        assert_eq!(USBCMD_HCRST, 0x0000_0002);
        assert_eq!(USBSTS_HCH, 0x0000_1000);
    }
}
