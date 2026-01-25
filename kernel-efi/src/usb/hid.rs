// Copyright 2025 The Rustux Authors
//
// USB HID Keyboard - Boot Protocol Implementation
//
// This module implements USB HID Boot Protocol keyboard parsing.
// Uses the standard 8-byte keyboard report format.


/// USB HID Boot Protocol keyboard report (always 8 bytes)
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct KeyboardReport {
    pub modifier: u8,
    pub reserved: u8,
    pub keycodes: [u8; 6],
}

/// Modifier key bits
const MODIFIER_LEFT_CTRL: u8 = 0x01;
const MODIFIER_LEFT_SHIFT: u8 = 0x02;
const MODIFIER_LEFT_ALT: u8 = 0x04;
const MODIFIER_LEFT_GUI: u8 = 0x08;
const MODIFIER_RIGHT_CTRL: u8 = 0x10;
const MODIFIER_RIGHT_SHIFT: u8 = 0x20;
const MODIFIER_RIGHT_ALT: u8 = 0x40;
const MODIFIER_RIGHT_GUI: u8 = 0x80;

/// USB HID keycode to ASCII translation table (256 entries)
const HID_TO_ASCII: [u8; 256] = [
    // 0x00-0x0F: Reserved keys + a-l
    0x00, 0x00, 0x00, 0x00, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',
    // 0x10-0x1F: m-z + reserved
    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', 0x00, 0x00,
    // 0x20-0x2F: 1-0, enter, escape, backspace, tab, space
    b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'\n', 0x1B, b'\x08', b'\t', b' ', 0x00,
    // 0x30-0x3F: Symbols
    b'-', b'=', b'[', b']', b'\\', 0x00, b';', b'\'', b'`', b',', b'.', b'/', 0x00, 0x00, 0x00, 0x00,
    // 0x40-0x4F: F1-F12 + reserved
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x50-0x5F: Keypad
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, b'/', b'*', b'-', b'+', b'\n', 0x00,
    // 0x60-0x6F: Keypad numbers
    b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'.', 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x70-0x7F: Reserved
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // 0x80-0xFF: Reserved (128 entries)
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Shift key mappings for number keys
const SHIFT_NUMBER: [u8; 10] = [
    b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')',
];

/// Shift key mappings for symbol keys
const SHIFT_SYMBOL: [u8; 12] = [
    b'_', b'+', b'{', b'}', b'|', 0x00, b':', b'"', 0x00, b'<', b'>', b'?',
];

/// Previous key state (for key press detection)
static mut PREVIOUS_KEYCODES: [u8; 6] = [0; 6];
static mut PREVIOUS_MODIFIER: u8 = 0;

/// Check if shift modifier is active
fn is_shift(modifier: u8) -> bool {
    (modifier & MODIFIER_LEFT_SHIFT != 0) || (modifier & MODIFIER_RIGHT_SHIFT != 0)
}

/// Convert HID keycode to ASCII with shift handling
fn keycode_to_ascii(keycode: u8, modifier: u8) -> u8 {
    if keycode as usize >= HID_TO_ASCII.len() {
        return 0;
    }

    let mut ascii = HID_TO_ASCII[keycode as usize];

    // Apply shift for letters and certain symbols
    if is_shift(modifier) {
        if ascii >= b'a' && ascii <= b'z' {
            ascii -= 32; // Convert to uppercase
        } else if ascii >= b'1' && ascii <= b'9' {
            ascii = SHIFT_NUMBER[(ascii - b'1') as usize];
        } else if ascii == b'0' {
            ascii = b')';
        } else {
            // Handle shifted symbols
            match ascii {
                b'-' => ascii = SHIFT_SYMBOL[0],
                b'=' => ascii = SHIFT_SYMBOL[1],
                b'[' => ascii = SHIFT_SYMBOL[2],
                b']' => ascii = SHIFT_SYMBOL[3],
                b'\\' => ascii = SHIFT_SYMBOL[4],
                b';' => ascii = SHIFT_SYMBOL[6],
                b'\'' => ascii = SHIFT_SYMBOL[7],
                b',' => ascii = SHIFT_SYMBOL[9],
                b'.' => ascii = SHIFT_SYMBOL[10],
                b'/' => ascii = SHIFT_SYMBOL[11],
                _ => {}
            }
        }
    }

    ascii
}

/// Detect key press (key is newly pressed, not held down)
fn is_key_press(keycode: u8) -> bool {
    unsafe {
        // Check if this keycode was NOT in the previous state
        !PREVIOUS_KEYCODES.contains(&keycode)
    }
}

/// Parse USB HID keyboard report and return ASCII character if available
pub fn parse_keyboard_report(report: &KeyboardReport) -> Option<char> {
    for keycode in &report.keycodes {
        if *keycode == 0 {
            continue;
        }

        // Only process newly pressed keys (detect edges, not held keys)
        if is_key_press(*keycode) {
            let ascii = keycode_to_ascii(*keycode, report.modifier);
            if ascii != 0 {
                return Some(ascii as char);
            }
        }
    }

    None
}

/// Poll USB keyboard for input
///
/// This function:
/// 1. Checks if xHCI controller and HID keyboard are available
/// 2. Reads keyboard data from the device module
/// 3. Parses the HID report and returns a character
pub fn poll_keyboard() -> Option<char> {
    unsafe {
        // Check if xHCI controller is available
        let _controller = super::xhci::controller()?;

        // Check if USB keyboard device is enumerated
        let _keyboard = super::device::get_hid_keyboard()?;

        // Read keyboard report from USB device
        let bytes_read = super::device::read_keyboard_report();
        if bytes_read == 0 {
            return None;
        }

        // Parse the report buffer
        let report = super::device::parse_report_buffer()?;

        // Get character from report
        let ch = parse_keyboard_report(&report);

        // Update previous key state
        update_key_state(&report);

        ch
    }
}

/// Update previous key state (call this after processing a report)
pub fn update_key_state(report: &KeyboardReport) {
    unsafe {
        PREVIOUS_KEYCODES = report.keycodes;
        PREVIOUS_MODIFIER = report.modifier;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyboard_report_size() {
        assert_eq!(core::mem::size_of::<KeyboardReport>(), 8);
    }

    #[test]
    fn test_parse_simple_key() {
        // Reset previous state
        unsafe {
            PREVIOUS_KEYCODES = [0; 6];
            PREVIOUS_MODIFIER = 0;
        }

        let report = KeyboardReport {
            modifier: 0,
            reserved: 0,
            keycodes: [0x04, 0, 0, 0, 0, 0], // HID keycode 0x04 = 'a'
        };
        let result = parse_keyboard_report(&report);
        assert_eq!(result, Some('a'));
    }

    #[test]
    fn test_parse_shifted_key() {
        // Reset previous state
        unsafe {
            PREVIOUS_KEYCODES = [0; 6];
            PREVIOUS_MODIFIER = 0;
        }

        let report = KeyboardReport {
            modifier: MODIFIER_LEFT_SHIFT,
            reserved: 0,
            keycodes: [0x04, 0, 0, 0, 0, 0], // HID keycode 0x04 = 'a' -> 'A'
        };
        let result = parse_keyboard_report(&report);
        assert_eq!(result, Some('A'));
    }

    #[test]
    fn test_key_repeat_detection() {
        // First press should return character
        unsafe {
            PREVIOUS_KEYCODES = [0; 6];
            PREVIOUS_MODIFIER = 0;
        }

        let report = KeyboardReport {
            modifier: 0,
            reserved: 0,
            keycodes: [0x04, 0, 0, 0, 0, 0],
        };
        let result1 = parse_keyboard_report(&report);
        assert_eq!(result1, Some('a'));

        // Second call with same keycodes should NOT return character (held key)
        let result2 = parse_keyboard_report(&report);
        assert_eq!(result2, None);
    }
}
