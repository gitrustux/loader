// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Minimal Shell Stub (Runtime Mode)
//!
//! This module provides a simple shell that runs after ExitBootServices.
//! It demonstrates the "I type text → I see text" objective.
//!
//! ## Features
//! - Print prompt "rustux> "
//! - Read line using keyboard buffer
//! - Echo line back to VGA console
//! - Handle backspace and enter
//! - Repeat forever
//!
//! ## Limitations
//! - NO command parsing or execution
//! - NO built-in commands
//! - NO arrow keys or history
//! - NO piping or redirection

/// Run the minimal shell loop
///
/// This function never returns. It continuously:
/// 1. Prints the prompt "rustux> "
/// 2. Reads a line from the keyboard
/// 3. Echoes the line back to the console
/// 4. Repeats
pub fn run_shell() -> ! {
    use crate::vga_console;
    use crate::keyboard;

    // Print shell header
    vga_console::set_color(11, 0); // Light cyan on black
    vga_console::puts("\n\n*** RUSTUX SHELL - RUNTIME MODE ***\n");
    vga_console::puts("Type text and press Enter to echo it back.\n");
    vga_console::puts("Press Ctrl+C to exit (not yet implemented).\n\n");

    let mut buffer = [0u8; 256];

    loop {
        // Print prompt
        vga_console::set_color(14, 0); // Yellow on black
        vga_console::puts("rustux> ");

        // Read line from keyboard (blocks until Enter)
        vga_console::set_color(15, 0); // White on black
        let n = unsafe { keyboard::read_line(&mut buffer) };

        // Echo the line back
        if n > 0 {
            vga_console::set_color(10, 0); // Light green on black
            vga_console::puts("Echo: ");

            // Convert buffer to string slice and print
            let input = unsafe { core::str::from_utf8_unchecked(&buffer[..n]) };
            vga_console::puts(input);

            // Print newline
            vga_console::puts("\n");
        } else {
            // Empty line - just print newline
            vga_console::puts("\n");
        }
    }
}

/// Initialize shell (stub for now)
pub fn init() {
    // Shell doesn't need initialization currently
    // The VGA console and keyboard are already initialized
}
