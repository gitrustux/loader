// Copyright 2025 The Rustux Authors
//
// Keyboard Driver
//
// Every keyboard that is found is used at the same time:
// - PS/2 keyboard (8042 controller, IRQ1, with polling as a fallback)
// - USB HID boot keyboard (polled)
//
// Both feed the same input. If neither is found the shell runs in
// display-only mode.

pub mod ps2;
pub mod usb;

use core::sync::atomic::{compiler_fence, Ordering};

/// Input buffer size (fixed, no heap)
const INPUT_BUFFER_SIZE: usize = 256;

/// PS/2 keyboard controller found and initialized
static mut PS2_ACTIVE: bool = false;
/// USB HID keyboard enumerated
static mut USB_ACTIVE: bool = false;

/// Whether the "no keyboard" message has been shown
static mut NO_KEYBOARD_WARNING_SHOWN: bool = false;

/// Number of keyboard interrupts taken (for the shell's `irq` command)
static mut IRQ_COUNT: u32 = 0;

/// Ring buffer of decoded characters.
///
/// Written by the IRQ handler (and by polling with interrupts disabled),
/// read by the shell. Fields shared with the interrupt handler are accessed
/// with volatile reads/writes so the compiler cannot cache them in a loop.
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

    /// Write a byte to the buffer (interrupt context or interrupts disabled)
    fn write(&mut self, byte: u8) -> bool {
        unsafe {
            let write_pos = core::ptr::read_volatile(&self.write_pos);
            let next_pos = (write_pos + 1) % INPUT_BUFFER_SIZE;
            if next_pos == core::ptr::read_volatile(&self.read_pos) {
                return false; // Buffer full
            }
            core::ptr::write_volatile(&mut self.data[write_pos], byte);
            compiler_fence(Ordering::Release);
            core::ptr::write_volatile(&mut self.write_pos, next_pos);
        }
        true
    }

    /// Read a byte from the buffer
    fn read(&mut self) -> Option<u8> {
        unsafe {
            let read_pos = core::ptr::read_volatile(&self.read_pos);
            if read_pos == core::ptr::read_volatile(&self.write_pos) {
                return None; // Buffer empty
            }
            compiler_fence(Ordering::Acquire);
            let byte = core::ptr::read_volatile(&self.data[read_pos]);
            core::ptr::write_volatile(&mut self.read_pos, (read_pos + 1) % INPUT_BUFFER_SIZE);
            Some(byte)
        }
    }

    /// Check if buffer has data
    #[allow(dead_code)]
    fn has_data(&self) -> bool {
        unsafe { core::ptr::read_volatile(&self.read_pos) != core::ptr::read_volatile(&self.write_pos) }
    }
}

/// Global input buffer
static mut INPUT_BUFFER: InputBuffer = InputBuffer::new();

/// Initialize the keyboard drivers.
///
/// USB and PS/2 are both set up when present; neither disables the other.
/// Must be called with interrupts disabled (the 8042 setup reads replies
/// from the data port that the IRQ handler would otherwise take).
pub fn init() {
    unsafe {
        NO_KEYBOARD_WARNING_SHOWN = false;

        if usb::init().is_ok() {
            USB_ACTIVE = true;
            crate::framebuffer::write_str("[USB KBD]\n");
        }

        if ps2::controller_present() {
            ps2::init_controller();
            PS2_ACTIVE = true;
            crate::framebuffer::write_str("[PS/2] ");
        }

        if !USB_ACTIVE && !PS2_ACTIVE {
            crate::framebuffer::write_str("[NO KEYBOARD] ");
        }
    }
}

/// Read a single character from any keyboard.
///
/// # Returns
/// * `Some(char)` - Character if available
/// * `None` - No character available
pub fn read_char() -> Option<char> {
    unsafe {
        if let Some(b) = INPUT_BUFFER.read() {
            return Some(b as char);
        }
        if PS2_ACTIVE {
            // Fallback for when IRQ1 is not delivered (e.g. wrong routing)
            ps2::poll();
            if let Some(b) = INPUT_BUFFER.read() {
                return Some(b as char);
            }
        }
        if USB_ACTIVE {
            return usb::read_char();
        }
        None
    }
}

/// Read a line from the keyboard until Enter is pressed, echoing input.
///
/// # Arguments
/// * `buffer` - Buffer to store the input string
///
/// # Returns
/// * Number of characters read (excluding newline)
///
/// If no keyboard is available, returns empty (shell continues without input).
pub fn read_line(buffer: &mut [u8]) -> usize {
    unsafe {
        if !PS2_ACTIVE && !USB_ACTIVE {
            // Only print the warning message once
            if !NO_KEYBOARD_WARNING_SHOWN {
                crate::framebuffer::write_str_color("\n[No keyboard attached - CLI running in display-only mode]\n",
                    crate::framebuffer::colors::encode(crate::framebuffer::colors::YELLOW));
                crate::framebuffer::write_str("Commands: help | clear | mem | kbd | ps | exit\n");
                NO_KEYBOARD_WARNING_SHOWN = true;
            }
            return 0; // Shell continues without input
        }
    }

    let mut count = 0;
    loop {
        if let Some(c) = read_char() {
            match c {
                '\n' => {
                    crate::framebuffer::write_str("\n");
                    break;
                }
                '\x08' => {
                    if count > 0 {
                        count -= 1;
                        crate::framebuffer::erase_prev_char();
                    }
                }
                ' '..='~' if count < buffer.len() => {
                    buffer[count] = c as u8;
                    count += 1;
                    let b = [c as u8];
                    crate::framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&b) });
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

/// Number of keyboard interrupts taken (wraps at 256)
pub fn get_irq_count() -> u8 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(IRQ_COUNT)) as u8 }
}

/// Flush the input buffer
pub fn flush() {
    unsafe {
        while INPUT_BUFFER.read().is_some() {}
    }
}

/// IRQ1 keyboard interrupt handler (called from the IDT stub)
#[no_mangle]
pub extern "C" fn keyboard_irq_handler() {
    unsafe {
        let n = core::ptr::read_volatile(core::ptr::addr_of!(IRQ_COUNT));
        core::ptr::write_volatile(core::ptr::addr_of_mut!(IRQ_COUNT), n.wrapping_add(1));
    }
    ps2::keyboard_irq_handler();
}

/// Internal: Write a byte to the input buffer (PS/2 IRQ handler and polling)
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
