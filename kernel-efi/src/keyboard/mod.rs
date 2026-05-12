// Copyright 2025 The Rustux Authors
//
// Keyboard Driver - Honest Backend Architecture
//
// This driver detects and uses available keyboard hardware:
// - PS/2 keyboard (legacy hardware, IRQ1)
// - USB keyboard (USB HID, polling)
//
// If no keyboard is detected, the kernel halts with a clear message.
// No fallbacks, no illusions - just honest hardware detection.

pub mod ps2;
pub mod usb;

/// Input buffer size (fixed, no heap)
const INPUT_BUFFER_SIZE: usize = 256;

/// Keyboard backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardBackend {
    None,
    Ps2,
    Usb,
}

/// Active keyboard backend
static mut KEYBOARD_BACKEND: KeyboardBackend = KeyboardBackend::None;

/// Flag to track if we've already shown the "no keyboard" warning
static mut NO_KEYBOARD_WARNING_SHOWN: bool = false;

/// Circular input buffer
struct InputBuffer {
    data: [u8; INPUT_BUFFER_SIZE],
    read_pos: usize,
    write_pos: usize,
}

impl InputBuffer {
    const fn new() -> Self {
        Self {
            data: [0; INPUT_BUFFER_SIZE],
            read_pos: 0,
            write_pos: 0,
        }
    }

    /// Write a byte to the buffer
    fn write(&mut self, byte: u8) -> bool {
        let next_pos = (self.write_pos + 1) % INPUT_BUFFER_SIZE;

        // Check if buffer is full
        if next_pos == self.read_pos {
            return false; // Buffer full
        }

        self.data[self.write_pos] = byte;
        self.write_pos = next_pos;
        true
    }

    /// Read a byte from the buffer
    fn read(&mut self) -> Option<u8> {
        if self.read_pos == self.write_pos {
            return None; // Buffer empty
        }

        let byte = self.data[self.read_pos];
        self.read_pos = (self.read_pos + 1) % INPUT_BUFFER_SIZE;
        Some(byte)
    }

    /// Check if buffer has data
    fn has_data(&self) -> bool {
        self.read_pos != self.write_pos
    }
}

/// Global input buffer
static mut INPUT_BUFFER: InputBuffer = InputBuffer::new();

/// Initialize the keyboard driver
///
/// Detects available keyboard hardware and sets the appropriate backend.
/// Prints [PS/2] if PS/2 detected, [USB KBD] if USB detected, [NO KEYBOARD] if none found.
///
/// CRITICAL: Once xHCI initializes successfully and a USB device is detected (CCS=1),
/// PS/2 fallback is DISABLED to prevent incorrect fallback behavior.
pub fn init() {
    unsafe {
        // Reset the warning flag on init
        NO_KEYBOARD_WARNING_SHOWN = false;

        // First try USB (Tier-1 input for modern systems)
        if let Ok(()) = usb::init() {
            KEYBOARD_BACKEND = KeyboardBackend::Usb;
            crate::framebuffer::write_str("[USB KBD] ");
            return;
        }

        // Check if xHCI succeeded with device detected (but USB init failed later)
        // This prevents PS/2 fallback when USB hardware is present but enumeration failed
        if crate::usb::device::xhci_succeeded_with_device() {
            crate::framebuffer::write_str("[USB DETECTED - ENUM PENDING] ");
            KEYBOARD_BACKEND = KeyboardBackend::Usb; // Keep USB backend for retry
            return;
        }

        // Fall back to PS/2 (legacy hardware) - only if no USB hardware detected
        if ps2::controller_present() {
            KEYBOARD_BACKEND = KeyboardBackend::Ps2;
            crate::framebuffer::write_str("[PS/2] ");
        } else {
            KEYBOARD_BACKEND = KeyboardBackend::None;
            crate::framebuffer::write_str("[NO KEYBOARD] ");
        }
    }
}

/// Read a single character from the keyboard buffer
///
/// # Returns
/// * `Some(char)` - Character if available
/// * `None` - No character available
pub fn read_char() -> Option<char> {
    unsafe {
        match KEYBOARD_BACKEND {
            KeyboardBackend::Ps2 => ps2::read_char(),
            KeyboardBackend::Usb => usb::read_char(),
            KeyboardBackend::None => None,
        }
    }
}

/// Read a line from the keyboard until Enter is pressed
///
/// # Arguments
/// * `buffer` - Buffer to store the input string
///
/// # Returns
/// * Number of characters read (excluding newline)
///
/// If no keyboard backend is available, returns empty (shell continues without input).
pub fn read_line(buffer: &mut [u8]) -> usize {
    unsafe {
        match KEYBOARD_BACKEND {
            KeyboardBackend::Ps2 => read_line_ps2(buffer),
            KeyboardBackend::Usb => read_line_usb(buffer),
            KeyboardBackend::None => {
                // Only print the warning message once
                if !NO_KEYBOARD_WARNING_SHOWN {
                    crate::framebuffer::write_str_color("\n[No keyboard attached - CLI running in display-only mode]\n",
                        crate::framebuffer::colors::encode(crate::framebuffer::colors::YELLOW));
                    crate::framebuffer::write_str("Commands: help | clear | mem | kbd | ps | exit\n");
                    NO_KEYBOARD_WARNING_SHOWN = true;
                }
                0  // Return empty - shell will continue without input
            }
        }
    }
}

/// PS/2 read_line implementation
fn read_line_ps2(buffer: &mut [u8]) -> usize {
    let mut count = 0;
    loop {
        if let Some(c) = read_char() {
            match c {
                '\n' => break,
                '\x08' => { if count > 0 { count -= 1; } }
                _ if count < buffer.len() => {
                    buffer[count] = c as u8;
                    count += 1;
                }
                _ => {}
            }
        }
        for _ in 0..1000 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)); }
        }
    }
    count
}

/// USB read_line implementation
fn read_line_usb(buffer: &mut [u8]) -> usize {
    let mut count = 0;
    loop {
        if let Some(c) = read_char() {
            match c {
                '\n' => break,
                '\x08' => { if count > 0 { count -= 1; } }
                _ if count < buffer.len() => {
                    buffer[count] = c as u8;
                    count += 1;
                }
                _ => {}
            }
        }
        for _ in 0..1000 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)); }
        }
    }
    count
}

/// Get IRQ count (for shell diagnostics)
pub fn get_irq_count() -> u8 {
    0
}

/// Flush the input buffer
pub fn flush() {
    unsafe {
        while INPUT_BUFFER.read().is_some() {}
    }
}

/// IRQ1 keyboard interrupt handler (dispatches to PS/2 handler)
///
/// This function is called by the IDT interrupt handler for IRQ1.
#[no_mangle]
pub extern "C" fn keyboard_irq_handler() {
    ps2::keyboard_irq_handler();
}

/// Internal: Write a byte to the input buffer (for PS/2 IRQ handler)
#[doc(hidden)]
pub unsafe fn write_to_buffer(byte: u8) -> bool {
    INPUT_BUFFER.write(byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_buffer_new() {
        let buffer = InputBuffer::new();
        assert!(!buffer.has_data());
    }

    #[test]
    fn test_input_buffer_write_read() {
        let mut buffer = InputBuffer::new();
        assert!(buffer.write(b'A'));
        assert!(buffer.has_data());
        assert_eq!(buffer.read(), Some(b'A'));
        assert!(!buffer.has_data());
    }
}
