// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Console I/O that works in both boot services and runtime modes
//!
//! After ExitBootServices, we can still use the text output protocol
//! if we saved the interface pointer before exiting.

use uefi::proto::console::text::Color;
use uefi::CStr16;

/// Saved text output protocol pointer (works after ExitBootServices)
static mut TEXT_OUTPUT_PTR: Option<usize> = None;

/// Saved text input protocol pointer (works after ExitBootServices)
static mut TEXT_INPUT_PTR: Option<usize> = None;

/// Flag indicating if we're in runtime mode (after ExitBootServices)
static mut IN_RUNTIME_MODE: bool = false;

/// Initialize the console layer - save protocol pointers before ExitBootServices
pub unsafe fn init_console() {
    use uefi::boot::{get_handle_for_protocol, open_protocol_exclusive};
    use uefi::proto::console::text::Output;
    use uefi::proto::console::text::Input;

    // Get and save stdout protocol pointer
    if let Ok(handle) = get_handle_for_protocol::<Output>() {
        if let Ok(protocol) = open_protocol_exclusive::<Output>(handle) {
            // Get a mutable reference and convert to raw pointer
            let interface_ptr: *const Output = &*protocol;
            TEXT_OUTPUT_PTR = Some(interface_ptr as usize);
            core::mem::forget(protocol); // Prevent cleanup - we're keeping the pointer
        }
    }

    // Get and save stdin protocol pointer
    if let Ok(handle) = get_handle_for_protocol::<Input>() {
        if let Ok(protocol) = open_protocol_exclusive::<Input>(handle) {
            let interface_ptr: *const Input = &*protocol;
            TEXT_INPUT_PTR = Some(interface_ptr as usize);
            core::mem::forget(protocol);
        }
    }

    IN_RUNTIME_MODE = false;
}

/// Mark that we've entered runtime mode (ExitBootServices called)
pub unsafe fn set_runtime_mode() {
    IN_RUNTIME_MODE = true;
}

/// Check if we're in runtime mode
pub fn is_runtime_mode() -> bool {
    unsafe { IN_RUNTIME_MODE }
}

/// Get the saved stdout protocol pointer
unsafe fn get_stdout() -> Option<*mut uefi::proto::console::text::Output> {
    TEXT_OUTPUT_PTR.map(|p| p as *mut uefi::proto::console::text::Output)
}

/// Get the saved stdin protocol pointer
unsafe fn get_stdin() -> Option<*mut uefi::proto::console::text::Input> {
    TEXT_INPUT_PTR.map(|p| p as *mut uefi::proto::console::text::Input)
}

/// Get a mutable reference to stdout (for internal use)
unsafe fn stdout_ref() -> Option<&'static mut uefi::proto::console::text::Output> {
    get_stdout().map(|p| &mut *p)
}

/// Get a mutable reference to stdin (for internal use)
unsafe fn stdin_ref() -> Option<&'static mut uefi::proto::console::text::Input> {
    get_stdin().map(|p| &mut *p)
}

/// Output a CStr16 string directly (for cstr16! macros)
pub unsafe fn output_cstr16(s: &CStr16) -> uefi::Result {
    if let Some(stdout) = get_stdout() {
        if !stdout.is_null() {
            return (&mut *stdout).output_string(s);
        }
    }
    // Fallback to boot services
    uefi::system::with_stdout(|stdout| stdout.output_string(s))
}

/// Output a string slice (converts to CStr16 internally)
pub fn output_str(s: &str) -> uefi::Result {
    // Convert to u16 slice
    let mut u16_vec = alloc::vec::Vec::new();
    for c in s.encode_utf16() {
        u16_vec.push(c);
    }
    u16_vec.push(0); // Null terminator

    unsafe {
        if let Some(stdout) = get_stdout() {
            if !stdout.is_null() {
                let stdout_ref = &mut *stdout;
                let cstr = CStr16::from_u16_with_nul(&u16_vec)
                    .map_err(|_| uefi::Status::INVALID_PARAMETER)?;
                return stdout_ref.output_string(&cstr);
            }
        }
    }

    // Fallback to boot services
    uefi::system::with_stdout(|stdout| {
        let cstr = CStr16::from_u16_with_nul(&u16_vec)
            .map_err(|_| uefi::Status::INVALID_PARAMETER)?;
        stdout.output_string(&cstr)
    })
}

/// Set the console color
pub fn set_color(fg: Color, bg: Color) -> uefi::Result {
    unsafe {
        if let Some(stdout) = get_stdout() {
            if !stdout.is_null() {
                return (&mut *stdout).set_color(fg, bg);
            }
        }
    }
    uefi::system::with_stdout(|stdout| stdout.set_color(fg, bg))
}

/// Clear the screen
pub fn clear_screen() -> uefi::Result {
    unsafe {
        if let Some(stdout) = get_stdout() {
            if !stdout.is_null() {
                return (&mut *stdout).clear();
            }
        }
    }
    uefi::system::with_stdout(|stdout| stdout.clear())
}

/// Enable/disable cursor
pub fn enable_cursor(enable: bool) -> uefi::Result {
    unsafe {
        if let Some(stdout) = get_stdout() {
            if !stdout.is_null() {
                return (&mut *stdout).enable_cursor(enable);
            }
        }
    }
    uefi::system::with_stdout(|stdout| stdout.enable_cursor(enable))
}

/// Set cursor position
pub fn set_cursor_position(col: usize, row: usize) -> uefi::Result {
    unsafe {
        if let Some(stdout) = get_stdout() {
            if !stdout.is_null() {
                return (&mut *stdout).set_cursor_position(col, row);
            }
        }
    }
    uefi::system::with_stdout(|stdout| stdout.set_cursor_position(col, row))
}

/// Read a key from input
pub fn read_key() -> Option<uefi::proto::console::text::Key> {
    unsafe {
        if let Some(stdin) = get_stdin() {
            if !stdin.is_null() {
                return (&mut *stdin).read_key().ok().flatten();
            }
        }
    }
    uefi::system::with_stdin(|stdin| stdin.read_key().ok().flatten())
}

/// Get the key event handle (for wait_for_key_event)
pub fn get_key_event() -> Option<uefi::Event> {
    unsafe {
        if let Some(stdin) = get_stdin() {
            if !stdin.is_null() {
                return (&mut *stdin).wait_for_key_event();
            }
        }
    }
    uefi::system::with_stdin(|stdin| stdin.wait_for_key_event())
}

/// Stall for a specified duration (microseconds)
pub fn stall(micros: u64) {
    if is_runtime_mode() {
        // In runtime mode, use busy-wait approximation
        unsafe {
            let cycles = micros * 3; // Rough approximation: 3 CPU cycles per microsecond
            for _ in 0..cycles {
                core::arch::asm!("nop");
            }
        }
    } else {
        uefi::boot::stall(core::time::Duration::from_micros(micros));
    }
}

/// Read a line of input (works in both modes)
pub fn read_line(buffer: &mut [u16]) -> usize {
    let mut pos = 0;

    loop {
        if pos >= buffer.len() - 1 {
            break;
        }

        // Wait for key press
        let key = if is_runtime_mode() {
            // Poll in runtime mode
            let mut attempts = 0;
            loop {
                if let Some(k) = read_key() {
                    break Some(k);
                }
                stall(1000); // 1ms polling delay
                attempts += 1;
                if attempts > 10000 {
                    break None; // Timeout after 10 seconds
                }
            }
        } else {
            // Use event-based waiting in boot services mode
            if let Some(key_event) = get_key_event() {
                let mut events = [key_event];
                let _ = uefi::boot::wait_for_event(&mut events);
                read_key()
            } else {
                None
            }
        };

        if let Some(key) = key {
            match key {
                uefi::proto::console::text::Key::Printable(c) => {
                    let c_val = u16::from(c);

                    // Handle Enter key
                    if c_val == 13 {  // Carriage Return
                        output_str("\r\n").ok();
                        break;
                    }
                    // Handle Backspace
                    else if c_val == 8 || c_val == 127 {
                        if pos > 0 {
                            pos -= 1;
                            buffer[pos] = 0;
                            output_str("\x08 \x08").ok();
                        }
                    }
                    // Handle regular characters
                    else if c_val >= 32 && c_val < 127 {
                        buffer[pos] = c_val;
                        pos += 1;
                        echo_char(c_val);
                    }
                }
                _ => {}
            }
        } else {
            stall(10 * 1000); // 10ms
        }
    }

    buffer[pos] = 0;
    pos
}

/// Echo a single character
fn echo_char(c: u16) {
    // Set color to bright white for input
    set_color(Color::White, Color::Blue).ok();

    // Build a single-char CStr16 and output it
    let u16_arr = [c, 0];

    unsafe {
        if let Some(stdout) = get_stdout() {
            if !stdout.is_null() {
                if let Ok(cstr) = CStr16::from_u16_with_nul(&u16_arr) {
                    let _ = (&mut *stdout).output_string(&cstr);
                    return;
                }
            }
        }
    }

    // Fallback to boot services
    uefi::system::with_stdout(|stdout| {
        if let Ok(cstr) = CStr16::from_u16_with_nul(&u16_arr) {
            let _ = stdout.output_string(&cstr);
        }
    });
}
