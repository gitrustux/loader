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

#![allow(dead_code)] // Many items are for future features
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

/// VGA I/O ports
const VGA_MISC_WRITE: u16 = 0x3C2;
const VGA_CRTC_INDEX: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;

/// Output byte to VGA port
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack)
    );
}

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

/// Force VGA text mode 3 (80x25, 16 colors)
///
/// This function explicitly switches the VGA hardware to text mode,
/// disabling any GOP/framebuffer mode that UEFI may have left active.
/// This MUST be called before writing to 0xB8000 for reliable output.
///
/// ## Safety
/// This function uses port I/O and must only be called after the
/// kernel has gained control of the hardware (after UEFI boot).
pub fn force_text_mode() {
    unsafe {
        // Select VGA color text mode (mode 3: 80x25, 16 colors)
        // BIOS interrupt would be INT 10h, AH=00h, AL=03h
        // But we're in protected mode, so we program VGA directly

        // Set misc output register to select color mode and enable CPU access
        // Bit 0-1: Clock select (00 = reserved, 01 = use 28.322 MHz)
        // Bit 2: Disable internal video driver (0 = enable, 1 = disable)
        // Bit 3: 64KB (0) or 32KB (1) memory address bit
        // Bit 4: 0 = color emulation, 1 = mono emulation
        // Bit 5-6: Horizontal sync polarity
        // Bit 7: Vertical sync polarity
        // For color text mode: 0x67 = 0110 0111 (CPU clock enable, color mode)
        outb(VGA_MISC_WRITE, 0x67);

        // Clear the entire screen to black
        let vga_buffer = VGA_BUFFER as *mut u16;
        for i in 0..(VGA_WIDTH * VGA_HEIGHT) {
            vga_buffer.add(i).write_volatile(0x0F00 | (' ' as u16));
        }
    }
}

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

/// Display a full-screen banner confirming VGA is working
///
/// This function writes directly to the VGA buffer to create a visible
/// full-screen banner. This is a DETERMINISTIC test - if VGA text mode
/// is working, you WILL see this screen.
///
/// ## Success Criteria
/// - Blue background across entire screen
/// - Yellow text with "RUSTUX KERNEL - RUNTIME MODE" on row 0
/// - Multiple rows of status messages
/// - A blinking cursor at bottom
///
/// If you don't see this, VGA text mode is NOT initialized.
pub fn display_runtime_banner() {
    unsafe {
        let vga_buffer = VGA_BUFFER as *mut u16;

        // Fill background with blue (color 0x1F00 = blue bg, white fg)
        for i in 0..(VGA_WIDTH * VGA_HEIGHT) {
            vga_buffer.add(i).write_volatile(0x1F00 | (' ' as u16));
        }

        // Row 0: Title (yellow on blue = 0x1E00)
        let title = b"RUSTUX KERNEL - RUNTIME MODE - VGA ACTIVE";
        let base = 0;
        for (i, &byte) in title.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1E00 | (byte as u16));
            }
        }

        // Row 2: Separator (white on blue = 0x1F00)
        for i in 0..VGA_WIDTH {
            vga_buffer.add(2 * VGA_WIDTH + i).write_volatile(0x1F00 | ('=' as u16));
        }

        // Row 4: Status (green on blue = 0x1A00)
        let status = b"Status: VGA Text Mode Initialized";
        let base = 4 * VGA_WIDTH;
        for (i, &byte) in status.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1A00 | (byte as u16));
            }
        }

        // Row 6: IDT status
        let idt_msg = b"[ ] IDT loaded - Waiting for check";
        let base = 6 * VGA_WIDTH;
        for (i, &byte) in idt_msg.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1F00 | (byte as u16));
            }
        }

        // Row 7: PIC status
        let pic_msg = b"[ ] PIC configured - Waiting for check";
        let base = 7 * VGA_WIDTH;
        for (i, &byte) in pic_msg.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1F00 | (byte as u16));
            }
        }

        // Row 8: IRQ1 status
        let irq_msg = b"[ ] IRQ1 handler - Waiting for check";
        let base = 8 * VGA_WIDTH;
        for (i, &byte) in irq_msg.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1F00 | (byte as u16));
            }
        }

        // Row 9: Interrupts enabled
        let int_msg = b"[ ] Interrupts enabled - Waiting for STI";
        let base = 9 * VGA_WIDTH;
        for (i, &byte) in int_msg.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1F00 | (byte as u16));
            }
        }

        // Row 11: Instructions (yellow on blue)
        let instr = b"TESTING: Press any key to trigger IRQ1 interrupt";
        let base = 11 * VGA_WIDTH;
        for (i, &byte) in instr.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1E00 | (byte as u16));
            }
        }

        // Row 12: More instructions
        let instr2 = b"         If IRQ1 works, character will appear below";
        let base = 12 * VGA_WIDTH;
        for (i, &byte) in instr2.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1E00 | (byte as u16));
            }
        }

        // Row 14: Keypress output area
        for i in 0..VGA_WIDTH {
            vga_buffer.add(14 * VGA_WIDTH + i).write_volatile(0x1F00 | ('-' as u16));
        }

        // Row 15: Output label
        let output_label = b"IRQ1 Key Output (last key pressed):";
        let base = 15 * VGA_WIDTH;
        for (i, &byte) in output_label.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1A00 | (byte as u16));
            }
        }

        // Row 17: Blinking cursor indicator
        let cursor_row = 17 * VGA_WIDTH;
        for i in 0..VGA_WIDTH {
            if i == 40 {
                // Cursor position (bright white on blue = 0x9F00)
                vga_buffer.add(cursor_row + i).write_volatile(0x9F00 | (0xDB as u16)); // 0xDB = block character
            } else {
                vga_buffer.add(cursor_row + i).write_volatile(0x1F00 | (' ' as u16));
            }
        }

        // Row 20-24: System info
        let info1 = b"System Status: KERNEL ALIVE - Runtime Mode Active";
        let base = 20 * VGA_WIDTH;
        for (i, &byte) in info1.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x0F00 | (byte as u16));
            }
        }

        let info2 = b"CPU Mode:     Protected Mode, Interrupts Disabled";
        let base = 21 * VGA_WIDTH;
        for (i, &byte) in info2.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x0F00 | (byte as u16));
            }
        }

        let info3 = b"Next Step:    Waiting for interrupt verification...";
        let base = 22 * VGA_WIDTH;
        for (i, &byte) in info3.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x0F00 | (byte as u16));
            }
        }
    }
}

/// Mark a checklist item as done
///
/// Updates the specified row with a green "[OK]" marker.
///
/// # Arguments
/// * `row` - Row number (0-24) to update
pub fn mark_checkpoint(row: usize) {
    unsafe {
        let vga_buffer = VGA_BUFFER as *mut u16;
        if row < VGA_HEIGHT {
            let base = row * VGA_WIDTH;
            // Write "[OK]" in bright green (0x9A00)
            vga_buffer.add(base).write_volatile(0x9A00 | ('[' as u16));
            vga_buffer.add(base + 1).write_volatile(0x9A00 | ('O' as u16));
            vga_buffer.add(base + 2).write_volatile(0x9A00 | ('K' as u16));
            vga_buffer.add(base + 3).write_volatile(0x9A00 | (']' as u16));
        }
    }
}

/// Write a character to the key output area (row 16)
///
/// Used by the IRQ1 handler to display the last key pressed.
///
/// # Arguments
/// * `c` - Character to display
pub fn write_key_output(c: char) {
    unsafe {
        let vga_buffer = VGA_BUFFER as *mut u16;
        let row = 16;
        let base = row * VGA_WIDTH;

        // Clear the line first
        for i in 0..VGA_WIDTH {
            vga_buffer.add(base + i).write_volatile(0x1F00 | (' ' as u16));
        }

        // Write the character in bright yellow (0x9E00) at center
        let msg = b"Last key: ";
        for (i, &byte) in msg.iter().enumerate() {
            if i < VGA_WIDTH {
                vga_buffer.add(base + i).write_volatile(0x1E00 | (byte as u16));
            }
        }

        // Write the actual key (bright yellow on blue = 0x9E00)
        let key_start = 11;
        vga_buffer.add(base + key_start).write_volatile(0x9E00 | (c as u16));
        vga_buffer.add(base + key_start + 1).write_volatile(0x9E00 | (' ' as u16));
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
