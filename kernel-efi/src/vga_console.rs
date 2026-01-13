// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! VGA Text Mode Console Driver
//!
//! This module provides a simple VGA text-mode console driver
//! that works after ExitBootServices. It requires NO heap allocation
//! and works entirely with stack-allocated buffers.
//!
//! ## Specifications
//! - Memory: 0xB8000 (VGA text buffer)
//! - Dimensions: 80 columns x 25 rows
//! - Colors: 16 foreground, 16 background (4-bit each)
//! - No heap allocation (no Vec, String, Box)
//!
//! ## Usage
//! ```rust
//! // Initialize console
//! vga_console::init();
//!
//! // Print string
//! vga_console::puts("Hello, world!\n");
//!
//! // Print single character
//! vga_console::putc('A');
//! ```

/// VGA text buffer base address
const VGA_BUFFER: u64 = 0xB8000;

/// VGA dimensions
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

/// Default color: White on black
const COLOR_DEFAULT: u16 = 0x0F00;

/// VGA Console state (static, no heap)
struct VgaConsoleState {
    /// Current cursor row (0-24)
    row: usize,
    /// Current cursor column (0-79)
    column: usize,
    /// Current color attribute
    color: u16,
}

impl VgaConsoleState {
    /// Create a new console state
    const fn new() -> Self {
        Self {
            row: 0,
            column: 0,
            color: COLOR_DEFAULT,
        }
    }
}

/// Global console state
static mut CONSOLE_STATE: VgaConsoleState = VgaConsoleState::new();

/// Initialize the VGA console
///
/// This function clears the screen and resets the cursor position.
/// It does NOT allocate any memory and is safe to call after ExitBootServices.
pub fn init() {
    unsafe {
        clear_screen();
        CONSOLE_STATE.row = 0;
        CONSOLE_STATE.column = 0;
        CONSOLE_STATE.color = COLOR_DEFAULT;
    }
}

/// Clear the entire screen
fn clear_screen() {
    unsafe {
        let vga_buffer = VGA_BUFFER as *mut u16;

        // Fill entire screen with spaces (with color attribute)
        for i in 0..(VGA_WIDTH * VGA_HEIGHT) {
            vga_buffer.add(i).write_volatile(0x0F00 | (' ' as u16));
        }
    }
}

/// Scroll the screen up by one row
///
/// This function moves all rows up by one position and clears
/// the bottom row. The cursor is moved to the beginning of
/// the bottom row.
fn scroll_up() {
    unsafe {
        let vga_buffer = VGA_BUFFER as *mut u16;

        // Move all rows up by one
        // Row N becomes Row N-1
        for row in 1..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                let src = vga_buffer.add(row * VGA_WIDTH + col);
                let dst = vga_buffer.add((row - 1) * VGA_WIDTH + col);
                dst.write_volatile(src.read_volatile());
            }
        }

        // Clear the bottom row
        let bottom_row = VGA_HEIGHT - 1;
        for col in 0..VGA_WIDTH {
            vga_buffer.add(bottom_row * VGA_WIDTH + col)
                .write_volatile(0x0F00 | (' ' as u16));
        }

        // Move cursor to beginning of bottom row
        CONSOLE_STATE.row = bottom_row;
        CONSOLE_STATE.column = 0;
    }
}

/// Write a single character to the console
///
/// This function handles:
/// - Newline (\n): Move to beginning of next row (with scroll)
/// - Carriage return (\r): Move to beginning of current row
/// - Backspace (\x08): Move cursor back one position (doesn't erase)
/// - Tab (\t): Move to next tab stop (every 8 columns)
/// - Regular characters: Write at current position
///
/// # Arguments
/// * `c` - Character to write
pub fn putc(c: char) {
    unsafe {
        match c {
            '\n' => {
                // Newline: move to next row
                CONSOLE_STATE.column = 0;
                CONSOLE_STATE.row += 1;

                // Scroll if we're past the bottom
                if CONSOLE_STATE.row >= VGA_HEIGHT {
                    scroll_up();
                }
            }
            '\r' => {
                // Carriage return: move to beginning of row
                CONSOLE_STATE.column = 0;
            }
            '\x08' => {
                // Backspace: move cursor back
                if CONSOLE_STATE.column > 0 {
                    CONSOLE_STATE.column -= 1;
                } else if CONSOLE_STATE.row > 0 {
                    CONSOLE_STATE.row -= 1;
                    CONSOLE_STATE.column = VGA_WIDTH - 1;
                }
            }
            '\t' => {
                // Tab: move to next tab stop (every 8 columns)
                CONSOLE_STATE.column = (CONSOLE_STATE.column + 8) & !7;
                if CONSOLE_STATE.column >= VGA_WIDTH {
                    putc('\n');
                }
            }
            _ => {
                // Regular character
                let vga_buffer = VGA_BUFFER as *mut u16;
                let offset = CONSOLE_STATE.row * VGA_WIDTH + CONSOLE_STATE.column;

                // Write character with color attribute
                vga_buffer.add(offset).write_volatile(
                    CONSOLE_STATE.color | (c as u16)
                );

                // Advance cursor
                CONSOLE_STATE.column += 1;

                // Wrap to next row if needed
                if CONSOLE_STATE.column >= VGA_WIDTH {
                    putc('\n');
                }
            }
        }
    }
}

/// Write a null-terminated string to the console
///
/// This function writes each character of the string to the console.
/// The string is NOT null-terminated in Rust - we just iterate over it.
///
/// # Arguments
/// * `s` - String slice to write
pub fn puts(s: &str) {
    for c in s.chars() {
        putc(c);
    }
}

/// Set the console color
///
/// # Arguments
/// * `foreground` - Foreground color (0-15)
/// * `background` - Background color (0-15)
///
/// # Color Values
/// - 0: Black
/// - 1: Blue
/// - 2: Green
/// - 3: Cyan
/// - 4: Red
/// - 5: Magenta
/// - 6: Brown
/// - 7: Light Gray
/// - 8: Dark Gray
/// - 9: Light Blue
/// - 10: Light Green
/// - 11: Light Cyan
/// - 12: Light Red
/// - 13: Light Magenta
/// - 14: Yellow
/// - 15: White
pub fn set_color(foreground: u8, background: u8) {
    unsafe {
        // Color attribute format: 0xBBFF where BB=background, FF=foreground
        CONSOLE_STATE.color = ((background as u16) << 12) | ((foreground as u16) << 8);
    }
}

/// Get current cursor position
///
/// # Returns
/// * `(row, column)` - Current cursor position
pub fn get_cursor() -> (usize, usize) {
    unsafe {
        (CONSOLE_STATE.row, CONSOLE_STATE.column)
    }
}

/// Set cursor position
///
/// # Arguments
/// * `row` - Row (0-24)
/// * `column` - Column (0-79)
pub fn set_cursor(row: usize, column: usize) {
    unsafe {
        if row < VGA_HEIGHT && column < VGA_WIDTH {
            CONSOLE_STATE.row = row;
            CONSOLE_STATE.column = column;
        }
    }
}

/// Write a number in hexadecimal format
///
/// # Arguments
/// * `value` - 64-bit value to print
pub fn put_hex(value: u64) {
    const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

    // Print "0x" prefix
    putc('0');
    putc('x');

    // Print each nibble
    for i in 0..16 {
        let nibble = (value >> (60 - i * 4)) & 0xF;
        putc(HEX_CHARS[nibble as usize] as char);
    }
}

/// Write a number in decimal format
///
/// # Arguments
/// * `value` - 64-bit value to print
pub fn put_dec(mut value: u64) {
    // Special case for 0
    if value == 0 {
        putc('0');
        return;
    }

    // Buffer for digits (max 20 digits for u64)
    let mut digits = [0u8; 20];
    let mut count = 0;

    // Extract digits
    while value > 0 && count < 20 {
        digits[count] = (value % 10) as u8;
        value /= 10;
        count += 1;
    }

    // Print in reverse order (most significant first)
    for i in (0..count).rev() {
        putc((b'0' + digits[i]) as char);
    }
}

/// Unit tests (compile-time only, since we can't run tests in UEFI easily)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(VGA_WIDTH, 80);
        assert_eq!(VGA_HEIGHT, 25);
        assert_eq!(VGA_BUFFER, 0xB8000);
    }

    #[test]
    fn test_console_state_new() {
        let state = VgaConsoleState::new();
        assert_eq!(state.row, 0);
        assert_eq!(state.column, 0);
        assert_eq!(state.color, COLOR_DEFAULT);
    }
}
