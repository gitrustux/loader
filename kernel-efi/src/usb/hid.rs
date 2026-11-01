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
/// HID keyboard usage ID (Usage Tables 1.12, page 0x07) to ASCII, US layout.
/// 0x1E-0x27 are 1-9,0; 0x28 Enter; 0x29 Esc; 0x2A Backspace; 0x2B Tab;
/// 0x54-0x63 the keypad.
const HID_TO_ASCII: [u8; 256] = [
    0x00, 0x00, 0x00, 0x00, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l',  // 0x00
    b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', b'1', b'2',  // 0x10
    b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'\n', 0x1B, 0x08, b'\t', b' ', b'-', b'=', b'[',  // 0x20
    b']', b'\\', b'\\', b';', b'\'', b'`', b',', b'.', b'/', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0x30
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0x40
    0x00, 0x00, 0x00, 0x00, b'/', b'*', b'-', b'+', b'\n', b'1', b'2', b'3', b'4', b'5', b'6', b'7',  // 0x50
    b'8', b'9', b'0', b'.', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0x60
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0x70
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0x80
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0x90
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0xA0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0xB0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0xC0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0xD0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0xE0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // 0xF0
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
                b'`' => ascii = b'~',
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
/// Characters decoded from reports but not yet returned. One report can
/// contain several newly pressed keys (fast typing overlaps key presses).
static mut PENDING: [u8; 16] = [0; 16];
static mut PENDING_HEAD: usize = 0;
static mut PENDING_LEN: usize = 0;

unsafe fn pending_push(c: u8) {
    if PENDING_LEN < PENDING.len() {
        PENDING[(PENDING_HEAD + PENDING_LEN) % PENDING.len()] = c;
        PENDING_LEN += 1;
    }
}

unsafe fn pending_pop() -> Option<char> {
    if PENDING_LEN == 0 {
        return None;
    }
    let c = PENDING[PENDING_HEAD];
    PENDING_HEAD = (PENDING_HEAD + 1) % PENDING.len();
    PENDING_LEN -= 1;
    Some(c as char)
}

/// Return the first newly pressed key in `report` (compared with the
/// previous report) and queue any others for later calls.
pub fn parse_keyboard_report(report: &KeyboardReport) -> Option<char> {
    unsafe {
        for keycode in &report.keycodes {
            // 0 = no key, 1 = rollover error (too many keys held)
            if *keycode <= 1 || !is_key_press(*keycode) {
                continue;
            }
            let ascii = keycode_to_ascii(*keycode, report.modifier);
            if ascii != 0 {
                pending_push(ascii);
            }
        }
        pending_pop()
    }
}

pub fn poll_keyboard() -> Option<char> {
    unsafe {
        if let Some(c) = pending_pop() {
            return Some(c);
        }

        let _controller = super::xhci::controller()?;

        let _keyboard = super::device::get_hid_keyboard()?;

        let bytes_read = super::device::read_keyboard_report();
        if bytes_read == 0 {
            return None;
        }

        let report = super::device::parse_report_buffer()?;

        // A rollover report (all keycodes 1) says nothing about which keys
        // are held; ignore it and keep the previous state.
        if report.keycodes[0] == 1 {
            return None;
        }

        let ch = parse_keyboard_report(&report);

        update_key_state(&report);

        ch
    }
}

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
