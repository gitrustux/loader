// Copyright 2025 The Rustux Authors
//
// PS/2 Keyboard Driver (Legacy)
//
// This module contains the PS/2 keyboard driver implementation.
// It uses the legacy 8042 controller and IRQ1 for keyboard input.

/// Keyboard data port (for reading data and sending device commands)
const KEYBOARD_DATA_PORT: u16 = 0x60;

/// Keyboard command/status port (for controller commands and status)
const KEYBOARD_COMMAND_PORT: u16 = 0x64;

/// Shift state
static mut SHIFT_PRESSED: bool = false;

/// US QWERTY scan code set 1 to ASCII translation table
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

/// Read controller status register (port 0x64)
unsafe fn controller_status() -> u8 {
    let status: u8;
    core::arch::asm!(
        "in al, dx",
        inlateout("dx") KEYBOARD_COMMAND_PORT => _,
        out("al") status,
        options(nomem, nostack)
    );
    status
}

/// Read byte from keyboard data port (0x60)
unsafe fn read_data_port() -> u8 {
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        inlateout("dx") KEYBOARD_DATA_PORT => _,
        out("al") value,
        options(nomem, nostack)
    );
    value
}

/// Flush output buffer (read any pending data)
unsafe fn flush_output_buffer() {
    let mut count = 0;
    while controller_status() & 0x01 != 0 && count < 128 {
        let _ = read_data_port();
        count += 1;
    }
}

/// Check if PS/2 controller is present
///
/// Returns true if the 8042 controller responds with something other than 0xFF.
pub unsafe fn controller_present() -> bool {
    let status = controller_status();
    status != 0xFF
}

/// Read a single character from the keyboard buffer
///
/// # Returns
/// * `Some(char)` - Character if available
/// * `None` - No character available
pub fn read_char() -> Option<char> {
    // Delegate to the parent module's buffer
    // This will be called by keyboard::read_char()
    None // PS/2 uses IRQ, characters come via interrupt
}

/// Handle a scan code (unified decode path for IRQ)
///
/// This function processes a single scan code and converts it to ASCII.
/// It handles both make codes (key press) and break codes (key release).
pub unsafe fn handle_scancode(scan_code: u8) {
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
                // Write to input buffer via parent module
                super::write_to_buffer(ascii);
            }
        }
    }
}

/// IRQ1 keyboard interrupt handler (internal)
///
/// This function is called by the parent module's keyboard_irq_handler.
/// It reads ALL pending scan codes from the keyboard and converts them to ASCII.
///
/// CRITICAL: Must drain the entire PS/2 buffer before sending EOI!
/// If any scancode remains unread, the controller will never raise another IRQ.
pub(crate) extern "C" fn keyboard_irq_handler() {
    unsafe {
        // Drain the entire PS/2 buffer before sending EOI
        loop {
            // Check controller status
            let status = controller_status();

            // Bit 0: output buffer full
            // If no data available, we're done draining
            if status & 0x01 == 0 {
                break;
            }

            // Ignore mouse data (bit 5 set)
            if status & 0x20 != 0 {
                // Read and discard mouse data
                let _ = read_data_port();
                continue;
            }

            // Ignore error conditions (timeout or parity error)
            if status & 0xC0 != 0 {
                // Read and discard error data
                let _ = read_data_port();
                continue;
            }

            // Read scan code from data port
            let scan_code = read_data_port();

            // Handle the scan code
            handle_scancode(scan_code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(KEYBOARD_DATA_PORT, 0x60);
        assert_eq!(KEYBOARD_COMMAND_PORT, 0x64);
    }

    #[test]
    fn test_scan_code_table_size() {
        assert_eq!(SCAN_CODE_TO_ASCII.len(), 128);
    }
}
