// Copyright 2025 The Rustux Authors
//
// USB Stack - Minimal Phase 7A Implementation
//
// This module provides USB HID keyboard support using polling.
// No IRQs, no scheduler dependency - works immediately after boot.

pub mod pci;
pub mod xhci;
pub mod ehci;
pub mod device;
pub mod hid;
pub mod trb;

/// USB error type
#[derive(Debug, Clone, Copy)]
pub enum UsbError {
    /// XHCI controller not found
    XhciNotFound,
    /// XHCI initialization failed
    XhciInitFailed,
    /// EHCI controller not found
    EhciNotFound,
    /// EHCI initialization failed
    EhciInitFailed,
    /// USB device not found
    DeviceNotFound,
    /// HID keyboard not found
    HidKeyboardNotFound,
    /// Timeout
    Timeout,
}

/// USB controller type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbControllerType {
    /// xHCI (USB 3.0)
    Xhci,
    /// EHCI (USB 2.0)
    Ehci,
    /// UHCI (USB 1.1)
    Uhci,
    /// OHCI (USB 1.1)
    Ohci,
}

/// XHCI controller information
#[derive(Debug, Clone, Copy)]
pub struct XhciInfo {
    pub mmio_base: u64,
}

/// EHCI controller information
#[derive(Debug, Clone, Copy)]
pub struct EhciInfo {
    pub mmio_base: u64,
}

/// Result type for USB operations
pub type UsbResult<T> = Result<T, UsbError>;

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

/// Helper: Print hex word
fn print_hex_dword(value: u32) {
    for i in (0..32).step_by(8).rev() {
        let byte = ((value >> i) & 0xFF) as u8;
        print_hex_byte(byte);
    }
}

/// Initialize USB HID keyboard
///
/// This is the main entry point for USB keyboard initialization.
/// It tries xHCI first (USB 3.0), then falls back to EHCI (USB 2.0).
/// Chain: PCI scan → controller init → device enumeration
///
/// # Initialization Order (CRITICAL!)
/// 1. Find USB controller via PCI scan (xHCI or EHCI)
/// 2. Map MMIO registers (get BAR0 from PCI config)
/// 3. Initialize controller
/// 4. Wait for controller ready (USB spec requires delays!)
/// 5. Enumerate USB devices
/// 6. Find HID keyboard
///
/// # Returns
/// - `Ok(())` - USB keyboard successfully initialized
/// - `Err(&'static str)` - No USB keyboard found or initialization failed
///
/// # Debug Pixels
/// - Pixel (15,1) green  = USB init success
/// - Pixel (15,1) red    = USB init failed
/// - Pixel (15,1) yellow = No USB controller found
pub fn init() -> Result<(), &'static str> {
    unsafe {
        crate::framebuffer::write_str("\n=== USB INIT DEBUG ===\n");

        // Visual indicator: USB init starting
        crate::framebuffer::put_pixel(15, 1, 0xFF, 0xFF, 0x00); // Yellow

        // DIAGNOSTIC: Check if PCI enumeration works at all
        crate::framebuffer::write_str("Step 1: Scanning PCI bus...\n");

        // Step 1: Try xHCI first (USB 3.0)
        crate::framebuffer::write_str("Step 2: Trying xHCI (USB 3.0)...\n");

        let controller_type = match xhci::init() {
            Ok(()) => {
                crate::framebuffer::write_str("USB: xHCI initialized\n");
                Some(ControllerType::Xhci)
            }
            Err(UsbError::XhciNotFound) => {
                crate::framebuffer::write_str("USB: No xHCI controller found\n");
                // Try EHCI instead
                crate::framebuffer::write_str("Step 2b: Trying EHCI (USB 2.0)...\n");
                match ehci::init() {
                    Ok(()) => {
                        crate::framebuffer::write_str("USB: EHCI initialized\n");
                        Some(ControllerType::Ehci)
                    }
                    Err(UsbError::EhciNotFound) => {
                        crate::framebuffer::write_str("USB: No EHCI controller found\n");
                        crate::framebuffer::put_pixel(15, 1, 0xFF, 0xFF, 0x00); // Yellow
                        return Err("No USB controller found (neither xHCI nor EHCI)");
                    }
                    Err(_) => {
                        crate::framebuffer::write_str("USB: EHCI init failed\n");
                        crate::framebuffer::put_pixel(15, 1, 0xFF, 0x00, 0x00); // Red
                        return Err("EHCI initialization failed");
                    }
                }
            }
            Err(_) => {
                crate::framebuffer::write_str("USB: xHCI init failed\n");
                crate::framebuffer::put_pixel(15, 1, 0xFF, 0x00, 0x00); // Red
                return Err("xHCI initialization failed");
            }
        };

        // Step 2: Verify controller and print diagnostics
        crate::framebuffer::write_str("Step 3: Checking controller...\n");

        match controller_type {
            Some(ControllerType::Xhci) => {
                if let Some(controller) = xhci::controller() {
                    crate::framebuffer::write_str("xHCI: MMIO base = 0x");
                    print_hex_dword(controller.mmio_base() as u32);
                    crate::framebuffer::write_str("\n");
                }
            }
            Some(ControllerType::Ehci) => {
                if let Some(controller) = ehci::controller() {
                    crate::framebuffer::write_str("EHCI: MMIO base = 0x");
                    print_hex_dword(controller.mmio_base() as u32);
                    crate::framebuffer::write_str("\n");
                }
            }
            None => {
                crate::framebuffer::write_str("USB: Controller not initialized\n");
                crate::framebuffer::put_pixel(15, 1, 0xFF, 0x00, 0x00); // Red
                return Err("Controller not available after init");
            }
        }

        // Wait for controller ready (USB spec requires delays!)
        crate::framebuffer::write_str("Step 4: Waiting for controller ready...\n");
        for _ in 0..100_000_000 {
            core::arch::asm!("nop");
        }

        // Check if controller is running
        let is_running = match controller_type {
            Some(ControllerType::Xhci) => {
                xhci::controller().map_or(false, |c| c.is_running())
            }
            Some(ControllerType::Ehci) => {
                ehci::controller().map_or(false, |c| c.is_running())
            }
            None => false,
        };

        if !is_running {
            crate::framebuffer::write_str("USB: Controller not running\n");
            crate::framebuffer::put_pixel(15, 1, 0xFF, 0x00, 0x00); // Red
            return Err("Controller not running");
        }

        crate::framebuffer::write_str("USB: Controller is running\n");

        // Step 3: Enumerate USB devices and find HID keyboard
        crate::framebuffer::write_str("Step 5: Enumerating USB devices...\n");

        match device::enumerate_hid_keyboard() {
            Ok(_keyboard) => {
                crate::framebuffer::write_str("USB: HID keyboard found\n");
                crate::framebuffer::put_pixel(15, 1, 0x00, 0xFF, 0x00); // Green = success
                Ok(())
            }
            Err(UsbError::DeviceNotFound) => {
                crate::framebuffer::write_str("USB: No device found on ports 1-4\n");
                crate::framebuffer::put_pixel(15, 1, 0xFF, 0xA5, 0x00); // Orange
                Err("No USB device found")
            }
            Err(UsbError::HidKeyboardNotFound) => {
                crate::framebuffer::write_str("USB: Device found but not a keyboard\n");
                crate::framebuffer::put_pixel(15, 1, 0xFF, 0xA5, 0x00); // Orange
                Err("No HID keyboard found")
            }
            Err(_e) => {
                crate::framebuffer::write_str("USB: Enumeration failed\n");
                crate::framebuffer::put_pixel(15, 1, 0xFF, 0x00, 0x00); // Red
                Err("USB enumeration failed")
            }
        }
    }
}

/// Internal controller type tracking
#[derive(Debug, Clone, Copy)]
enum ControllerType {
    Xhci,
    Ehci,
}

/// Read a character from USB keyboard
///
/// Uses polling-based approach (no interrupts).
/// Polls the USB controller for new HID reports and converts to ASCII.
///
/// # HID Boot Protocol Report Format (8 bytes):
/// - [0]: Modifier keys (Ctrl, Shift, Alt, GUI)
/// - [1]: Reserved (must be 0)
/// - [2-7]: Key codes (up to 6 simultaneous keys)
///
/// # Returns
/// - `Some(char)` - ASCII character if key pressed
/// - `None` - No key pressed or no USB keyboard
///
/// # Debug Pixels
/// - Pixel (16,1) cyan = USB data received
pub fn read_char() -> Option<char> {
    unsafe {
        // Poll the USB HID keyboard
        match hid::poll_keyboard() {
            Some(ch) => {
                // Visual indicator: USB data received
                crate::framebuffer::put_pixel(16, 1, 0x8B, 0xE9, 0xFD); // Cyan
                Some(ch)
            }
            None => None,
        }
    }
}

/// Check if USB keyboard is available
///
/// # Returns
/// - `true` - USB keyboard has been enumerated and is ready
/// - `false` - No USB keyboard available
pub fn is_keyboard_available() -> bool {
    unsafe {
        xhci::controller().is_some() && device::has_hid_keyboard()
    }
}

// Helper: Convert UsbError to &'static str
impl UsbError {
    pub fn as_str(&self) -> &'static str {
        match self {
            UsbError::XhciNotFound => "No xHCI controller found",
            UsbError::XhciInitFailed => "xHCI initialization failed",
            UsbError::EhciNotFound => "No EHCI controller found",
            UsbError::EhciInitFailed => "EHCI initialization failed",
            UsbError::DeviceNotFound => "No USB device found",
            UsbError::HidKeyboardNotFound => "No HID keyboard found",
            UsbError::Timeout => "USB operation timed out",
        }
    }
}
