// Copyright 2025 The Rustux Authors
//
// PCI Scan for USB Controllers
//
// Scans PCI configuration space for USB controllers (xHCI, EHCI, UHCI, OHCI).
// Returns the best available controller with MMIO base address from BAR0.
// Priority: xHCI > EHCI > UHCI/OHCI

use crate::usb::{XhciInfo, EhciInfo, UsbControllerType, UsbError};

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
const PCI_PROG_IF_UHCI: u8 = 0x00;
const PCI_PROG_IF_OHCI: u8 = 0x10;
const PCI_PROG_IF_EHCI: u8 = 0x20;
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

/// Enable bus mastering and memory space access for a PCI device
///
/// CRITICAL: This must be called after finding a device.
/// Sets Bus Master Enable (bit 2) and Memory Space Enable (bit 1) in the command register.
unsafe fn enable_bus_mastering(bus: u8, device: u8, function: u8) {
    let command = pci_read_config(bus, device, function, 0x04); // Command register

    // Set Bus Master Enable (bit 2) and Memory Space Enable (bit 1)
    let new_command = command | 0x06;

    pci_write_config(bus, device, function, 0x04, new_command);
}

/// Write 32-bit value to PCI configuration space
unsafe fn pci_write_config(bus: u8, device: u8, function: u8, reg: u8, value: u32) {
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

    // Write data to CONFIG_DATA
    core::arch::asm!(
        "out dx, eax",
        inlateout("dx") PCI_CONFIG_DATA => _,
        in("eax") value,
        options(nomem, nostack)
    );
}

/// Scan PCI buses for any USB controller
///
/// Returns the best available USB controller (xHCI > EHCI > others).
/// Scans all buses (0-255), devices (0-31), functions (0-7).
pub fn scan_for_usb_controller() -> Result<(UsbControllerType, u64), UsbError> {
    unsafe {
        crate::framebuffer::write_str("PCI: Scanning for USB controllers...\n");

        // Track the best controller found (xHCI > EHCI > others)
        let mut best_controller: Option<(UsbControllerType, u64, u8, u8, u8)> = None;

        for bus in 0u8..=255 {
            let vendor_id = pci_read_config(bus, 0, 0, PCI_VENDOR_ID) as u16;
            if vendor_id == 0xFFFF {
                if bus == 0 {
                    crate::framebuffer::write_str("PCI: Bus 0 has no devices\n");
                }
                continue;
            }

            let max_device = if bus == 0 { 8 } else { 32 };

            for device in 0u8..max_device {
                for function in 0u8..8 {
                    let vendor_id = pci_read_config(bus, device, function, PCI_VENDOR_ID) as u16;
                    if vendor_id == 0xFFFF {
                        continue;
                    }

                    let class = pci_read_config(bus, device, function, PCI_CLASS_CODE) as u8;
                    let subclass = pci_read_config(bus, device, function, PCI_SUBCLASS_CODE) as u8;
                    let prog_if = pci_read_config(bus, device, function, PCI_PROG_IF) as u8;

                    // USB controller: Class 0x0C, Subclass 0x03
                    if class == PCI_CLASS_SERIAL_BUS && subclass == PCI_SUBCLASS_USB {
                        let controller_type = match prog_if {
                            PCI_PROG_IF_XHCI => {
                                crate::framebuffer::write_str("PCI: Found xHCI (USB 3.0) controller\n");
                                UsbControllerType::Xhci
                            }
                            PCI_PROG_IF_EHCI => {
                                crate::framebuffer::write_str("PCI: Found EHCI (USB 2.0) controller\n");
                                UsbControllerType::Ehci
                            }
                            PCI_PROG_IF_UHCI => {
                                crate::framebuffer::write_str("PCI: Found UHCI (USB 1.1) controller\n");
                                UsbControllerType::Uhci
                            }
                            PCI_PROG_IF_OHCI => {
                                crate::framebuffer::write_str("PCI: Found OHCI (USB 1.1) controller\n");
                                UsbControllerType::Ohci
                            }
                            _ => {
                                crate::framebuffer::write_str("PCI: Found unknown USB controller\n");
                                continue;
                            }
                        };

                        // Get BAR0 MMIO base
                        if let Some(mmio_base) = get_bar0_mmio(bus, device, function) {
                            // Print MMIO address
                            crate::framebuffer::write_str("PCI: MMIO base = 0x");
                            let mut base = mmio_base;
                            for _ in 0..8 {
                                let nibble = (base & 0xF0000000) >> 28;
                                let c = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
                                crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
                                base <<= 4;
                            }
                            crate::framebuffer::write_str("\n");

                            // Enable bus mastering
                            enable_bus_mastering(bus, device, function);

                            // Update best controller (priority: xHCI > EHCI > others)
                            if best_controller.is_none()
                                || matches!(controller_type, UsbControllerType::Xhci)
                                || (matches!(controller_type, UsbControllerType::Ehci)
                                    && !matches!(best_controller, Some((UsbControllerType::Xhci, _, _, _, _))))
                            {
                                best_controller = Some((controller_type, mmio_base, bus, device, function));
                            }

                            // If we found xHCI, we can stop searching (it's the best)
                            if matches!(controller_type, UsbControllerType::Xhci) {
                                crate::framebuffer::write_str("PCI: Using xHCI controller\n");
                                return Ok((controller_type, mmio_base));
                            }
                        }
                    }
                }
            }

            if bus == 0 {
                break;
            }
        }

        if let Some((controller_type, mmio_base, _, _, _)) = best_controller {
            match controller_type {
                UsbControllerType::Xhci => {
                    crate::framebuffer::write_str("PCI: Using xHCI controller\n");
                }
                UsbControllerType::Ehci => {
                    crate::framebuffer::write_str("PCI: Using EHCI controller\n");
                }
                _ => {
                    crate::framebuffer::write_str("PCI: Using legacy USB controller (limited support)\n");
                }
            }
            return Ok((controller_type, mmio_base));
        }

        crate::framebuffer::write_str("PCI: No compatible USB controller found\n");
        Err(UsbError::XhciNotFound) // Will be translated to appropriate error
    }
}

/// Scan PCI buses for XHCI controller (legacy, for backward compatibility)
///
/// Returns XHCI controller MMIO base address if found.
pub fn scan_for_xhci() -> Result<XhciInfo, UsbError> {
    match scan_for_usb_controller() {
        Ok((UsbControllerType::Xhci, mmio_base)) => Ok(XhciInfo { mmio_base }),
        Ok(_) => Err(UsbError::XhciNotFound),
        Err(e) => Err(e),
    }
}

/// Scan PCI buses for EHCI controller
///
/// Returns EHCI controller MMIO base address if found.
pub fn scan_for_ehci() -> Result<EhciInfo, UsbError> {
    match scan_for_usb_controller() {
        Ok((UsbControllerType::Ehci, mmio_base)) => Ok(EhciInfo { mmio_base }),
        Ok((controller_type, _)) => {
            crate::framebuffer::write_str("PCI: Found ");
            match controller_type {
                UsbControllerType::Xhci => crate::framebuffer::write_str("xHCI"),
                UsbControllerType::Uhci => crate::framebuffer::write_str("UHCI"),
                UsbControllerType::Ohci => crate::framebuffer::write_str("OHCI"),
                _ => crate::framebuffer::write_str("unknown"),
            }
            crate::framebuffer::write_str(" instead of EHCI\n");
            Err(UsbError::EhciNotFound)
        }
        Err(e) => Err(e),
    }
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
