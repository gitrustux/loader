// Copyright 2025 The Rustux Authors
//
// PCI Scan for XHCI Controller
//
// Scans PCI configuration space for USB 3.0 xHCI controllers.
// Returns MMIO base address from BAR0.

use crate::usb::XhciInfo;
use crate::usb::UsbError;

/// PCI configuration access mechanism 1 (standard on x86_64)
const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// PCI address format for CONFIG_ADDRESS
/// Bits [31:24] - Reserved (must be 0)
/// Bits [23:16] - Bus number
/// Bits [15:11] - Device number
/// Bits [10:8]  - Function number
/// Bits [7:2]   - Register number
/// Bits [1:0]   - Reserved (must be 00)

/// PCI class codes for USB controllers
const PCI_CLASS_SERIAL_BUS: u8 = 0x0C;
const PCI_SUBCLASS_USB: u8 = 0x03;
const PCI_PROG_IF_XHCI: u8 = 0x30;

/// PCI registers
const PCI_VENDOR_ID: u8 = 0x00;
const PCI_DEVICE_ID: u8 = 0x02;
const PCI_CLASS_CODE: u8 = 0x08;
const PCI_SUBCLASS_CODE: u8 = 0x09;
const PCI_PROG_IF: u8 = 0x0A;
const PCI_BAR0: u8 = 0x10;

/// BAR0 type bits
const BAR_TYPE_MASK: u32 = 0x0F;
const BAR_TYPE_64BIT: u32 = 0x04;
const BAR_TYPE_MEMORY: u32 = 0x00;

/// Read 32-bit value from PCI configuration space
unsafe fn pci_read_config(bus: u8, device: u8, function: u8, reg: u8) -> u32 {
    let address = 0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((reg as u32) & 0xFC);

    // Write address to CONFIG_ADDRESS
    core::arch::asm!(
        "out dx, eax",
        in("dx") PCI_CONFIG_ADDRESS,
        in("eax") address,
        options(nomem, nostack)
    );

    // Read data from CONFIG_DATA
    let value: u32;
    core::arch::asm!(
        "in eax, dx",
        inlateout("dx") PCI_CONFIG_DATA => _,
        out("eax") value,
        options(nomem, nostack)
    );

    value
}

/// Get BAR0 MMIO base address from PCI config space
unsafe fn get_bar0_mmio(bus: u8, device: u8, function: u8) -> Option<u64> {
    let bar0_low = pci_read_config(bus, device, function, PCI_BAR0);

    // Check if this is a memory BAR (bit 0 = 0)
    if bar0_low & 0x01 != 0 {
        return None; // I/O space, not MMIO
    }

    let bar_type = bar0_low & BAR_TYPE_MASK;

    match bar_type {
        BAR_TYPE_MEMORY => {
            // 32-bit BAR
            let mmio_base = bar0_low & 0xFFFF_FFF0;
            if mmio_base == 0 {
                None
            } else {
                Some(mmio_base as u64)
            }
        }
        BAR_TYPE_64BIT => {
            // 64-bit BAR - read upper 32 bits from BAR1
            let bar1 = pci_read_config(bus, device, function, PCI_BAR0 + 4);
            let mmio_base = ((bar1 as u64) << 32) | ((bar0_low & 0xFFFF_FFF0) as u64);
            if mmio_base == 0 {
                None
            } else {
                Some(mmio_base)
            }
        }
        _ => None,
    }
}

/// Scan PCI buses for XHCI controller
///
/// Returns XHCI controller MMIO base address if found.
/// Scans all buses (0-255), devices (0-31), functions (0-7).
pub fn scan_for_xhci() -> Result<XhciInfo, UsbError> {
    unsafe {
        // Scan buses 0-255 (typical is just 0, but scan all for completeness)
        for bus in 0u8..=255 {
            // Check if bus exists by reading vendor ID of device 0 function 0
            let vendor_id = pci_read_config(bus, 0, 0, PCI_VENDOR_ID) as u16;
            if vendor_id == 0xFFFF {
                continue; // No device on this bus
            }

            // Scan devices 0-31
            for device in 0u8..32 {
                // Scan functions 0-7
                for function in 0u8..8 {
                    let vendor_id = pci_read_config(bus, device, function, PCI_VENDOR_ID) as u16;
                    if vendor_id == 0xFFFF {
                        continue; // No function here
                    }

                    // Check class code
                    let class = pci_read_config(bus, device, function, PCI_CLASS_CODE) as u8;
                    let subclass = pci_read_config(bus, device, function, PCI_SUBCLASS_CODE) as u8;
                    let prog_if = pci_read_config(bus, device, function, PCI_PROG_IF) as u8;

                    if class == PCI_CLASS_SERIAL_BUS
                        && subclass == PCI_SUBCLASS_USB
                        && prog_if == PCI_PROG_IF_XHCI
                    {
                        // Found XHCI controller - get BAR0 MMIO base
                        if let Some(mmio_base) = get_bar0_mmio(bus, device, function) {
                            crate::framebuffer::write_str("[USB KBD] ");
                            return Ok(XhciInfo { mmio_base });
                        }
                    }
                }
            }
        }
    }

    Err(UsbError::XhciNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pci_constants() {
        assert_eq!(PCI_CLASS_SERIAL_BUS, 0x0C);
        assert_eq!(PCI_SUBCLASS_USB, 0x03);
        assert_eq!(PCI_PROG_IF_XHCI, 0x30);
    }
}
