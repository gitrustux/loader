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
const PCI_HEADER_TYPE: u8 = 0x0E;
const PCI_BAR0: u8 = 0x10;
const PCI_BAR1: u8 = 0x14;
const PCI_BAR2: u8 = 0x18;
const PCI_BAR3: u8 = 0x1C;
const PCI_BAR4: u8 = 0x20;
const PCI_BAR5: u8 = 0x24;

/// PCI header type bits
const HEADER_TYPE_MULTIFUNCTION: u8 = 0x80;

/// PCI bridge class/subclass
const PCI_CLASS_BRIDGE: u8 = 0x06;
const PCI_SUBCLASS_PCI_PCI: u8 = 0x04;

/// PCI bridge registers
const PCI_PRIMARY_BUS: u8 = 0x18;
const PCI_SECONDARY_BUS: u8 = 0x19;
const PCI_SUBORDINATE_BUS: u8 = 0x1A;

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

/// Helper: Print hex byte
fn print_hex_byte(value: u8) {
    let high = (value >> 4) & 0x0F;
    let low = value & 0x0F;
    let high_c = if high < 10 { b'0' + high } else { b'A' + (high - 10) };
    let low_c = if low < 10 { b'0' + low } else { b'A' + (low - 10) };
    unsafe {
        crate::framebuffer::write_str(core::str::from_utf8(&[high_c]).unwrap());
        crate::framebuffer::write_str(core::str::from_utf8(&[low_c]).unwrap());
    }
}

/// Print all PCI device details (full diagnostic dump)
unsafe fn dump_pci_device(bus: u8, device: u8, function: u8,
                          vendor_id: u16, device_id: u16,
                          class: u8, subclass: u8, prog_if: u8, header_type: u8) {
    // Print bus:device.function
    crate::framebuffer::write_str("PCI: ");
    print_hex_byte(bus);
    crate::framebuffer::write_str(":");
    print_hex_byte(device);
    crate::framebuffer::write_str(".");
    match function {
        0 => crate::framebuffer::write_str("0"),
        1 => crate::framebuffer::write_str("1"),
        2 => crate::framebuffer::write_str("2"),
        3 => crate::framebuffer::write_str("3"),
        4 => crate::framebuffer::write_str("4"),
        5 => crate::framebuffer::write_str("5"),
        6 => crate::framebuffer::write_str("6"),
        7 => crate::framebuffer::write_str("7"),
        _ => crate::framebuffer::write_str("?"),
    }

    // Print vendor/device IDs
    crate::framebuffer::write_str(" vendor=");
    print_hex_byte((vendor_id >> 8) as u8);
    print_hex_byte(vendor_id as u8);
    crate::framebuffer::write_str(" device=");
    print_hex_byte((device_id >> 8) as u8);
    print_hex_byte(device_id as u8);

    // Print class/subclass/prog_if
    crate::framebuffer::write_str(" class=");
    print_hex_byte(class);
    crate::framebuffer::write_str(" sub=");
    print_hex_byte(subclass);
    crate::framebuffer::write_str(" if=");
    print_hex_byte(prog_if);

    // Print header type
    crate::framebuffer::write_str(" hdr=");
    print_hex_byte(header_type);

    // Flag special devices
    if class == PCI_CLASS_SERIAL_BUS && subclass == PCI_SUBCLASS_USB {
        crate::framebuffer::write_str(" [USB]");
    } else if class == PCI_CLASS_BRIDGE && subclass == PCI_SUBCLASS_PCI_PCI {
        crate::framebuffer::write_str(" [BRIDGE]");
        // Print bridge bus numbers
        let primary = pci_read_config(bus, device, function, PCI_PRIMARY_BUS) as u8;
        let secondary = pci_read_config(bus, device, function, PCI_SECONDARY_BUS) as u8;
        let subordinate = pci_read_config(bus, device, function, PCI_SUBORDINATE_BUS) as u8;
        crate::framebuffer::write_str(" buses=");
        let c = b'0' + primary;
        crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
        crate::framebuffer::write_str("-");
        let c = b'0' + secondary;
        crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
        crate::framebuffer::write_str("-");
        let c = b'0' + subordinate;
        crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
    }

    // Flag Intel devices (likely USB)
    if vendor_id == 0x8086 {
        crate::framebuffer::write_str(" [INTEL]");
    }

    crate::framebuffer::write_str("\n");

    // For USB candidates, also print BARs
    if class == PCI_CLASS_SERIAL_BUS && subclass == PCI_SUBCLASS_USB {
        for bar_num in 0..6 {
            let bar_reg = match bar_num {
                0 => PCI_BAR0, 1 => PCI_BAR1, 2 => PCI_BAR2,
                3 => PCI_BAR3, 4 => PCI_BAR4, _ => PCI_BAR5,
            };
            let bar = pci_read_config(bus, device, function, bar_reg);
            if bar != 0 {
                crate::framebuffer::write_str("  BAR");
                let c = b'0' + bar_num;
                crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
                crate::framebuffer::write_str(" = 0x");
                for i in (0..32).step_by(4).rev() {
                    let nibble = ((bar >> i) & 0xF) as u8;
                    let c = if nibble < 10 { b'0' + nibble } else { b'A' + (nibble - 10) };
                    crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
                }
                if bar & 0x01 != 0 {
                    crate::framebuffer::write_str(" [I/O]");
                } else {
                    crate::framebuffer::write_str(" [MEM]");
                }
                crate::framebuffer::write_str("\n");
            }
        }
    }
}

/// Scan PCI buses for any USB controller
///
/// Returns the best available USB controller (xHCI > EHCI > others).
/// Scans all buses (0-255), devices (0-31), functions (0-7).
pub fn scan_for_usb_controller() -> Result<(UsbControllerType, u64), UsbError> {
    unsafe {
        crate::framebuffer::write_str("PCI: Scanning for USB controllers...\n");

        // DEBUG: Test if PCI reads work at all by reading bus 0, device 0
        let test_vendor = pci_read_config(0, 0, 0, PCI_VENDOR_ID);
        crate::framebuffer::write_str("PCI: Test read bus0:dev0:fn0 vendor = 0x");
        for i in (0..32).step_by(4).rev() {
            let nibble = ((test_vendor >> i) & 0xF) as u8;
            let c = if nibble < 10 { b'0' + nibble } else { b'A' + (nibble - 10) };
            crate::framebuffer::write_str(core::str::from_utf8(&[c]).unwrap());
        }
        crate::framebuffer::write_str("\n");

        // Track the best controller found (xHCI > EHCI > others)
        let mut best_controller: Option<(UsbControllerType, u64, u8, u8, u8)> = None;
        let mut devices_found = 0u32;
        let mut buses_with_devices = 0u8;

        for bus in 0u8..=255 {
            let max_device = if bus == 0 { 8 } else { 32 };
            let mut bus_has_devices = false;

            for device in 0u8..max_device {
                for function in 0u8..8 {
                    let vendor_id = pci_read_config(bus, device, function, PCI_VENDOR_ID) as u16;
                    if vendor_id == 0xFFFF {
                        continue;
                    }

                    if !bus_has_devices {
                        buses_with_devices += 1;
                        bus_has_devices = true;
                    }

                    devices_found += 1;  // Count all PCI devices found

                    // Read PCI config DWORDs and extract fields properly
                    // Offset 0x00: Vendor ID (bits 0-15) + Device ID (bits 16-31)
                    let vendor_device = pci_read_config(bus, device, function, 0x00);
                    let vendor_id = (vendor_device & 0xFFFF) as u16;
                    let device_id = ((vendor_device >> 16) & 0xFFFF) as u16;

                    // Offset 0x08: Class (bits 24-31) + Subclass (bits 16-23) + Prog IF (bits 8-15) + Revision (bits 0-7)
                    let class_reg = pci_read_config(bus, device, function, 0x08);
                    let class = ((class_reg >> 24) & 0xFF) as u8;
                    let subclass = ((class_reg >> 16) & 0xFF) as u8;
                    let prog_if = ((class_reg >> 8) & 0xFF) as u8;

                    // Offset 0x0C: Cache Line + Latency + Header Type + BIST
                    let header_reg = pci_read_config(bus, device, function, 0x0C);
                    let header_type = ((header_reg >> 16) & 0xFF) as u8;

                    // DIAGNOSTIC: Dump all PCI devices
                    dump_pci_device(bus, device, function, vendor_id, device_id, class, subclass, prog_if, header_type);

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
                                crate::framebuffer::write_str("PCI: Found USB controller with unknown prog_if = 0x");
                                print_hex_byte(prog_if);
                                crate::framebuffer::write_str(", attempting to use\n");
                                // Try to use it anyway as a generic USB controller
                                UsbControllerType::Xhci  // Assume xHCI for unknown
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
        }

        // Scan all 255 buses - USB controllers can be on higher bus numbers
        // No need to break early, the loop handles this efficiently by skipping
        // buses/devices that don't exist (vendor_id == 0xFFFF)

        // DEBUG: Report total PCI devices found
        crate::framebuffer::write_str("PCI: Total devices found = ");
        if devices_found > 9999 {
            crate::framebuffer::write_str("MANY\n");
        } else {
            // Simple decimal conversion for devices_found
            let mut temp = devices_found;
            let mut digits = [0u8; 5];
            let mut len = 0;
            if temp == 0 {
                digits[len] = b'0';
                len += 1;
            } else {
                while temp > 0 && len < 5 {
                    digits[len] = b'0' + (temp % 10) as u8;
                    temp /= 10;
                    len += 1;
                }
            }
            // Print in reverse order
            for i in (0..len).rev() {
                crate::framebuffer::write_str(core::str::from_utf8(&[digits[i]]).unwrap());
            }
            crate::framebuffer::write_str("\n");
        }

        // DEBUG: Report buses scanned
        crate::framebuffer::write_str("PCI: Buses with devices = ");
        let mut temp = buses_with_devices;
        let mut digits = [0u8; 3];
        let mut len = 0;
        if temp == 0 {
            digits[len] = b'0';
            len += 1;
        } else {
            while temp > 0 && len < 3 {
                digits[len] = b'0' + (temp % 10) as u8;
                temp /= 10;
                len += 1;
            }
        }
        for i in (0..len).rev() {
            crate::framebuffer::write_str(core::str::from_utf8(&[digits[i]]).unwrap());
        }
        crate::framebuffer::write_str("\n");

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
