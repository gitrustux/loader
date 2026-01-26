// Copyright 2025 The Rustux Authors
//
// USB Keyboard Adapter
//
// This module provides the adapter layer for USB HID keyboard support.
// It interfaces with the usb::device and usb::hid modules for polling keyboard input.

/// Initialize USB keyboard support
///
/// Attempts to initialize the xHCI controller and find a USB HID keyboard.
/// Returns Ok(()) if successful, Err if no USB keyboard found.
pub fn init() -> Result<(), &'static str> {
    unsafe {
        // Call top-level USB init (includes debug output)
        crate::usb::init()?;

        Ok(())
    }
}

/// Read a single character from the USB keyboard
///
/// Polls the USB HID keyboard for input and returns the character if available.
/// Returns None if no character is available.
pub fn read_char() -> Option<char> {
    // Poll the USB HID keyboard
    crate::usb::hid::poll_keyboard()
}

/// Check if USB keyboard is available
pub fn is_keyboard_available() -> bool {
    crate::usb::device::has_hid_keyboard()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_exists() {
        // Just verify the function exists
        // Actual initialization requires hardware
    }

    #[test]
    fn test_is_keyboard_available_exists() {
        // Function exists for runtime checking
        assert!(!is_keyboard_available()); // Should be false without hardware
    }
}
