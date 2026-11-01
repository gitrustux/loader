// Copyright 2025 The Rustux Authors
//
// PS/2 Keyboard Driver (8042 controller)
//
// The keyboard is set up explicitly at boot instead of relying on what the
// firmware left behind: IRQ1 enabled, scancode translation on (so the
// keyboard's set 2 codes arrive as set 1), keyboard port and scanning
// enabled. Bytes are taken by the IRQ1 handler; `poll` reads them directly
// when interrupts are not delivered.

/// Keyboard data port (for reading data and sending device commands)
const KEYBOARD_DATA_PORT: u16 = 0x60;

/// Keyboard command/status port (for controller commands and status)
const KEYBOARD_COMMAND_PORT: u16 = 0x64;

/// Status register bits
const STATUS_OUTPUT_FULL: u8 = 0x01;
const STATUS_INPUT_FULL: u8 = 0x02;
const STATUS_AUX_DATA: u8 = 0x20;
const STATUS_ERRORS: u8 = 0xC0; // timeout, parity

/// Controller configuration byte bits
const CFG_KBD_IRQ: u8 = 0x01;
const CFG_AUX_IRQ: u8 = 0x02;
const CFG_KBD_CLOCK_OFF: u8 = 0x10;
const CFG_AUX_CLOCK_OFF: u8 = 0x20;
const CFG_TRANSLATE: u8 = 0x40;

/// Shift / Caps Lock / E0-prefix state
static mut SHIFT_PRESSED: bool = false;
static mut CAPS_LOCK: bool = false;
static mut EXTENDED: bool = false;

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

/// Shifted characters for the non-letter keys of scan code set 1
fn shifted(scan_code: u8) -> u8 {
    match scan_code {
        0x02 => b'!', 0x03 => b'@', 0x04 => b'#', 0x05 => b'$', 0x06 => b'%',
        0x07 => b'^', 0x08 => b'&', 0x09 => b'*', 0x0A => b'(', 0x0B => b')',
        0x0C => b'_', 0x0D => b'+', 0x1A => b'{', 0x1B => b'}', 0x27 => b':',
        0x28 => b'"', 0x29 => b'~', 0x2B => b'|', 0x33 => b'<', 0x34 => b'>',
        0x35 => b'?',
        _ => 0,
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
    value
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

/// Read controller status register (port 0x64)
unsafe fn controller_status() -> u8 {
    inb(KEYBOARD_COMMAND_PORT)
}

/// Read byte from keyboard data port (0x60)
unsafe fn read_data_port() -> u8 {
    inb(KEYBOARD_DATA_PORT)
}

/// Wait until the controller can accept a byte. Port reads take about a
/// microsecond each, so the bound is in the 100 ms range.
unsafe fn wait_input_empty() -> bool {
    for _ in 0..100_000u32 {
        if controller_status() & STATUS_INPUT_FULL == 0 {
            return true;
        }
    }
    false
}

/// Wait for a byte from the controller or keyboard.
unsafe fn wait_output(timeout: u32) -> Option<u8> {
    for _ in 0..timeout {
        if controller_status() & STATUS_OUTPUT_FULL != 0 {
            return Some(read_data_port());
        }
    }
    None
}

unsafe fn send_command(cmd: u8) -> bool {
    if !wait_input_empty() {
        return false;
    }
    outb(KEYBOARD_COMMAND_PORT, cmd);
    true
}

unsafe fn send_data(byte: u8) -> bool {
    if !wait_input_empty() {
        return false;
    }
    outb(KEYBOARD_DATA_PORT, byte);
    true
}

/// Controller command that returns one byte
unsafe fn command_with_reply(cmd: u8) -> Option<u8> {
    if !send_command(cmd) {
        return None;
    }
    wait_output(200_000)
}

/// Flush output buffer (read any pending data)
unsafe fn flush_output_buffer() {
    let mut count = 0;
    while controller_status() & STATUS_OUTPUT_FULL != 0 && count < 128 {
        let _ = read_data_port();
        count += 1;
    }
}

/// Run `f` with interrupts disabled, restoring the previous state.
unsafe fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let flags: u64;
    core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem));
    let r = f();
    if flags & 0x200 != 0 {
        core::arch::asm!("sti", options(nomem, nostack));
    }
    r
}

/// Check if PS/2 controller is present
///
/// A missing controller reads back as 0xFF.
pub unsafe fn controller_present() -> bool {
    controller_status() != 0xFF
}

fn print_hex_byte(v: u8) {
    let h = b"0123456789ABCDEF";
    let b = [h[(v >> 4) as usize], h[(v & 0xF) as usize]];
    crate::framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&b) });
}

fn print_reply(label: &str, v: Option<u8>) {
    crate::framebuffer::write_str(label);
    match v {
        Some(b) => print_hex_byte(b),
        None => crate::framebuffer::write_str("--"),
    }
}

/// Set up the 8042 controller and keyboard (OSDev "8042 PS/2 Controller"
/// initialisation sequence, without the mouse). Prints one diagnostic line:
///
///   PS/2: test=55 cfg=XX->YY port=00 kbd=FA
///
/// test = controller self-test (55 = OK), cfg = configuration byte before
/// and after, port = keyboard port test (00 = OK), kbd = keyboard's reply
/// to "enable scanning" (FA = ACK). "--" means no reply.
pub unsafe fn init_controller() {
    without_interrupts(|| {
        // Stop both ports while reconfiguring
        send_command(0xAD);
        send_command(0xA7);
        flush_output_buffer();

        let cfg_before = command_with_reply(0x20);

        // Controller self-test; some controllers reset their configuration
        // byte here, so the configuration is written afterwards.
        let self_test = command_with_reply(0xAA);

        // Keyboard on, IRQ1 on, translation to set 1 on; mouse off.
        let cfg = (cfg_before.unwrap_or(0) | CFG_KBD_IRQ | CFG_TRANSLATE | CFG_AUX_CLOCK_OFF)
            & !(CFG_AUX_IRQ | CFG_KBD_CLOCK_OFF);
        if send_command(0x60) {
            send_data(cfg);
        }
        let cfg_after = command_with_reply(0x20);

        let port_test = command_with_reply(0xAB);

        // Enable the keyboard port, then tell the keyboard to send scancodes
        send_command(0xAE);
        let mut kbd = None;
        for _ in 0..3 {
            flush_output_buffer();
            if !send_data(0xF4) {
                break;
            }
            kbd = wait_output(200_000);
            if kbd != Some(0xFE) {
                break; // anything but "resend"
            }
        }
        flush_output_buffer();

        crate::framebuffer::write_str("PS/2:");
        print_reply(" test=", self_test);
        print_reply(" cfg=", cfg_before);
        print_reply("->", cfg_after);
        print_reply(" port=", port_test);
        print_reply(" kbd=", kbd);
        crate::framebuffer::write_str("\n");
    });
}

/// Handle a scan code (set 1, as delivered with translation on)
pub unsafe fn handle_scancode(scan_code: u8) {
    if scan_code == 0xE0 {
        EXTENDED = true;
        return;
    }
    if scan_code == 0xE1 {
        // Pause key sequence: ignore
        return;
    }
    let released = scan_code & 0x80 != 0;
    let make_code = scan_code & 0x7F;

    if EXTENDED {
        EXTENDED = false;
        // Keypad Enter and keypad '/'; ignore the rest (arrows, the fake
        // shifts sent around Print Screen, right Ctrl/Alt, ...)
        if !released {
            match make_code {
                0x1C => { super::write_to_buffer(b'\n'); }
                0x35 => { super::write_to_buffer(b'/'); }
                _ => {}
            }
        }
        return;
    }

    match make_code {
        0x2A | 0x36 => {
            SHIFT_PRESSED = !released;
            return;
        }
        0x3A => {
            if !released {
                CAPS_LOCK = !CAPS_LOCK;
            }
            return;
        }
        _ => {}
    }
    if released {
        return;
    }

    let mut ascii = match make_code {
        0x37 => b'*', // keypad *
        0x4A => b'-', // keypad -
        0x4E => b'+', // keypad +
        _ => SCAN_CODE_TO_ASCII[make_code as usize],
    };
    if ascii.is_ascii_lowercase() {
        if SHIFT_PRESSED != CAPS_LOCK {
            ascii -= 32;
        }
    } else if SHIFT_PRESSED {
        let s = shifted(make_code);
        if s != 0 {
            ascii = s;
        }
    }
    if ascii != 0 {
        super::write_to_buffer(ascii);
    }
}

/// Read and decode every byte waiting in the controller.
///
/// Must run with interrupts disabled or from the IRQ handler.
unsafe fn drain() {
    for _ in 0..64 {
        let status = controller_status();
        if status & STATUS_OUTPUT_FULL == 0 {
            break;
        }
        let byte = read_data_port();
        if status & (STATUS_AUX_DATA | STATUS_ERRORS) != 0 {
            continue; // mouse data or a transmission error
        }
        handle_scancode(byte);
    }
}

/// Polling fallback: decode pending bytes without waiting for IRQ1.
pub unsafe fn poll() {
    if controller_status() & STATUS_OUTPUT_FULL == 0 {
        return;
    }
    without_interrupts(|| drain());
}

/// IRQ1 keyboard interrupt handler (internal)
///
/// Reads ALL pending bytes: the controller raises IRQ1 again only after
/// the output buffer has been emptied.
pub(crate) extern "C" fn keyboard_irq_handler() {
    unsafe { drain() }
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
