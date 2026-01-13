// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! PS/2 Keyboard Driver
//!
//! This module provides a simple PS/2 keyboard driver that works
//! after ExitBootServices. It handles scan codes from the keyboard
//! and converts them to ASCII characters.
//!
//! ## Hardware
//! - Data port: 0x60
//! - Command port: 0x64
//! - IRQ: IRQ1 (interrupt 33)
//!
//! ## Supported Keys
//! - Letters (a-z, A-Z)
//! - Numbers (0-9)
//! - Space
//! - Backspace
//! - Enter
//! - Comma, Dash, Period
//!
//! ## Limitations
//! - No Shift modifier support
//! - No special keys (F1-F12, arrows, etc.)
//! - US QWERTY layout only

/// Keyboard data port
const KEYBOARD_DATA_PORT: u16 = 0x60;

/// Keyboard command/status port
const KEYBOARD_COMMAND_PORT: u16 = 0x64;

/// Input buffer size (fixed, no heap)
const INPUT_BUFFER_SIZE: usize = 256;

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

    /// Get number of bytes available
    fn available(&self) -> usize {
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            INPUT_BUFFER_SIZE - self.read_pos + self.write_pos
        }
    }
}

/// Global input buffer
static mut INPUT_BUFFER: InputBuffer = InputBuffer::new();

/// Shift state (for future use)
static mut SHIFT_PRESSED: bool = false;

/// US QWERTY scan code set 1 to ASCII translation table
/// This maps scan codes to ASCII characters (unshifted)
const SCAN_CODE_TO_ASCII: &[u8; 128] = &[
    0,      // 0x00: Unknown
    0,      // 0x01: Esc (ignored for now)
    b'1',   // 0x02: 1
    b'2',   // 0x03: 2
    b'3',   // 0x04: 3
    b'4',   // 0x05: 4
    b'5',   // 0x06: 5
    b'6',   // 0x07: 6
    b'7',   // 0x08: 7
    b'8',   // 0x09: 8
    b'9',   // 0x0A: 9
    b'0',   // 0x0B: 0
    b'-',   // 0x0C: -
    b'=',   // 0x0D: =
    b'\x08', // 0x0E: Backspace
    b'\t',  // 0x0F: Tab
    b'q',   // 0x10: Q
    b'w',   // 0x11: W
    b'e',   // 0x12: E
    b'r',   // 0x13: R
    b't',   // 0x14: T
    b'y',   // 0x15: Y
    b'u',   // 0x16: U
    b'i',   // 0x17: I
    b'o',   // 0x18: O
    b'p',   // 0x19: P
    b'[',   // 0x1A: [
    b']',   // 0x1B: ]
    b'\n',  // 0x1C: Enter
    0,      // 0x1D: Left Ctrl (ignored)
    b'a',   // 0x1E: A
    b's',   // 0x1F: S
    b'd',   // 0x20: D
    b'f',   // 0x21: F
    b'g',   // 0x22: G
    b'h',   // 0x23: H
    b'j',   // 0x24: J
    b'k',   // 0x25: K
    b'l',   // 0x26: L
    b';',   // 0x27: ;
    b'\'',  // 0x28: '
    b'`',   // 0x29: `
    0,      // 0x2A: Left Shift (ignored)
    b'\\',  // 0x2B: \
    b'z',   // 0x2C: Z
    b'x',   // 0x2D: X
    b'c',   // 0x2E: C
    b'v',   // 0x2F: V
    b'b',   // 0x30: B
    b'n',   // 0x31: N
    b'm',   // 0x32: M
    b',',   // 0x33: ,
    b'.',   // 0x34: .
    b'/',   // 0x35: /
    0,      // 0x36: Right Shift (ignored)
    0,      // 0x37: Print Screen (ignored)
    0,      // 0x38: Alt (ignored)
    b' ',   // 0x39: Space
    0,      // 0x3A: Caps Lock (ignored)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x3B-0x44: F1-F10 (ignored)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x45-0x4E: Various (ignored)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x4F-0x58: Various (ignored)
    0,      // 0x59
    0,      // 0x5A
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x5B-0x64: Various (ignored)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x65-0x6E: Various (ignored)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x6F-0x78: Various (ignored)
    0, 0, 0, 0, 0, 0, 0, // 0x79-0x7F: Various (ignored)
];

/// Initialize the PS/2 keyboard driver
///
/// This function enables the keyboard IRQ and prepares the driver
/// for input. It must be called after the IDT is initialized.
pub fn init() {
    unsafe {
        // Reset input buffer
        INPUT_BUFFER = InputBuffer::new();
        SHIFT_PRESSED = false;

        // Read scan code from data port to clear any pending data
        let _ = read_data_port();
    }
}

/// IRQ1 keyboard interrupt handler
///
/// This function should be called from the IDT interrupt handler for IRQ1.
/// It reads the scan code from the keyboard and converts it to ASCII.
#[no_mangle]
pub extern "C" fn keyboard_irq_handler() {
    unsafe {
        // Read scan code from data port
        let scan_code = read_data_port();

        // Check if this is a release code (0x80 prefix)
        if scan_code & 0x80 != 0 {
            // Release code - extract the actual scan code
            let make_code = scan_code & 0x7F;

            // Check if shift is being released
            if make_code == 0x2A || make_code == 0x36 {
                SHIFT_PRESSED = false;
            }
        } else {
            // Make code - key press
            // Check if shift is being pressed
            if scan_code == 0x2A || scan_code == 0x36 {
                SHIFT_PRESSED = true;
                return;
            }

            // Convert scan code to ASCII
            if (scan_code as usize) < SCAN_CODE_TO_ASCII.len() {
                let mut ascii = SCAN_CODE_TO_ASCII[scan_code as usize];

                // Apply shift for letters (convert to uppercase)
                if SHIFT_PRESSED && ascii >= b'a' && ascii <= b'z' {
                    ascii -= 32; // Convert to uppercase
                }

                // Ignore null bytes (unsupported keys)
                if ascii != 0 {
                    // Write to input buffer
                    INPUT_BUFFER.write(ascii);
                }
            }
        }
    }
}

/// Read a byte from the keyboard data port
///
/// # Safety
/// This function uses port I/O and must be called with proper synchronization.
unsafe fn read_data_port() -> u8 {
    let port = KEYBOARD_DATA_PORT as *mut u8;
    port.read_volatile()
}

/// Check if keyboard has data available
pub fn has_input() -> bool {
    unsafe { INPUT_BUFFER.has_data() }
}

/// Get number of bytes available in input buffer
pub fn available() -> usize {
    unsafe { INPUT_BUFFER.available() }
}

/// Read a single character from the keyboard buffer
///
/// # Returns
/// * `Some(char)` - Character if available
/// * `None` - No character available
pub fn read_char() -> Option<char> {
    unsafe {
        INPUT_BUFFER.read().map(|b| b as char)
    }
}

/// Read a string from the keyboard until Enter is pressed
///
/// This function blocks until Enter is pressed and returns the
/// input string (excluding the newline character).
///
/// # Arguments
/// * `buffer` - Buffer to store the input string
///
/// # Returns
/// * Number of characters read (excluding newline)
pub fn read_line(buffer: &mut [u8]) -> usize {
    let mut count = 0;

    loop {
        if let Some(c) = read_char() {
            match c {
                '\n' => {
                    // Enter key - end of line
                    break;
                }
                '\x08' => {
                    // Backspace
                    if count > 0 {
                        count -= 1;
                    }
                }
                _ if count < buffer.len() => {
                    // Regular character
                    buffer[count] = c as u8;
                    count += 1;
                }
                _ => {
                    // Buffer full - ignore
                }
            }
        }

        // Small delay to prevent busy-waiting
        for _ in 0..1000 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)); }
        }
    }

    count
}

/// Flush the input buffer
pub fn flush() {
    unsafe {
        while INPUT_BUFFER.read().is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(KEYBOARD_DATA_PORT, 0x60);
        assert_eq!(KEYBOARD_COMMAND_PORT, 0x64);
        assert_eq!(INPUT_BUFFER_SIZE, 256);
    }

    #[test]
    fn test_scan_code_table_size() {
        assert_eq!(SCAN_CODE_TO_ASCII.len(), 128);
    }

    #[test]
    fn test_input_buffer_new() {
        let buffer = InputBuffer::new();
        assert!(!buffer.has_data());
        assert_eq!(buffer.available(), 0);
    }
}
