// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Rustux CLI Shell (Runtime Mode)
//!
//! This module provides a simple command-line interface that runs after ExitBootServices.
//!
//! ## Features
//! - Built-in commands: help, clear, echo, info, mem
//! - Command parsing with argument support
//! - Framebuffer console output
//! - Keyboard input (basic PS/2 driver)
//!
//! ## Limitations
//! - No external program loading (yet)
//! - No pipes or redirection
//! - No background jobs
//! - No command history
//! - Basic keyboard only (no shift/modifiers)

use crate::framebuffer;
use crate::keyboard;
use crate::mouse;
use alloc::vec::Vec;

/// Command buffer size
const COMMAND_BUFFER_SIZE: usize = 256;

/// Built-in command handler type
type CommandHandler = fn(&[&str]);

/// Built-in command descriptor
struct Command {
    name: &'static str,
    description: &'static str,
    handler: CommandHandler,
}

/// Built-in commands table
const BUILT_IN_COMMANDS: &[Command] = &[
    Command {
        name: "help",
        description: "Show this help message",
        handler: cmd_help,
    },
    Command {
        name: "clear",
        description: "Clear the screen",
        handler: cmd_clear,
    },
    Command {
        name: "echo",
        description: "Echo arguments to console",
        handler: cmd_echo,
    },
    Command {
        name: "info",
        description: "Show system information",
        handler: cmd_info,
    },
    Command {
        name: "mem",
        description: "Show memory information",
        handler: cmd_mem,
    },
    Command {
        name: "mouse",
        description: "Show mouse status",
        handler: cmd_mouse,
    },
];

/// Run the Rustux CLI shell
///
/// This function never returns. It continuously:
/// 1. Prints the prompt "rustux> "
/// 2. Reads a line from the keyboard
/// 3. Parses and executes the command
/// 4. Repeats
pub fn run_shell() -> ! {
    // Print shell header
    framebuffer::write_str("\n\n*** RUSTUX SHELL v0.1 ***\n");
    framebuffer::write_str("Type 'help' for available commands.\n\n");

    let mut buffer = [0u8; COMMAND_BUFFER_SIZE];

    loop {
        // Print prompt
        framebuffer::write_str("rustux> ");

        // Read line from keyboard (blocks until Enter)
        let n = keyboard::read_line(&mut buffer);

        // Skip empty lines
        if n == 0 {
            framebuffer::write_str("\n");
            continue;
        }

        // Convert buffer to string slice
        let input = unsafe { core::str::from_utf8_unchecked(&buffer[..n]) };

        // Parse command
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            framebuffer::write_str("\n");
            continue;
        }

        let command_name = parts[0];
        let args = &parts[1..];

        // Find and execute command
        let mut found = false;
        for cmd in BUILT_IN_COMMANDS {
            if cmd.name == command_name {
                (cmd.handler)(args);
                found = true;
                break;
            }
        }

        if !found {
            framebuffer::write_str("Unknown command: ");
            framebuffer::write_str(command_name);
            framebuffer::write_str("\nType 'help' for available commands.\n");
        }

        // Always print newline after command
        framebuffer::write_str("\n");
    }
}

/// Initialize shell (stub for now)
pub fn init() {
    // Shell doesn't need initialization currently
    // The framebuffer console and keyboard are already initialized
}

// =========================================================
// BUILT-IN COMMANDS
// =========================================================

/// Command: help - Show available commands
fn cmd_help(_args: &[&str]) {
    framebuffer::write_str("Available commands:\n");
    for cmd in BUILT_IN_COMMANDS {
        framebuffer::write_str("  ");
        framebuffer::write_str(cmd.name);
        framebuffer::write_str(" - ");
        framebuffer::write_str(cmd.description);
        framebuffer::write_str("\n");
    }
}

/// Command: clear - Clear the screen
fn cmd_clear(_args: &[&str]) {
    framebuffer::clear();
}

/// Command: echo - Echo arguments to console
fn cmd_echo(args: &[&str]) {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            framebuffer::write_str(" ");
        }
        framebuffer::write_str(arg);
    }
}

/// Command: info - Show system information
fn cmd_info(_args: &[&str]) {
    framebuffer::write_str("Rustux Kernel v0.1.0\n");
    framebuffer::write_str("Architecture: x86_64\n");
    framebuffer::write_str("Mode: Runtime (UEFI boot services exited)\n");
    framebuffer::write_str("Console: Framebuffer text mode\n");
}

/// Command: mem - Show memory information
fn cmd_mem(_args: &[&str]) {
    use crate::runtime;

    framebuffer::write_str("Memory Information:\n");

    // Check if allocator is initialized
    if let Some(allocator) = unsafe { runtime::get_kernel_allocator() } {
        let available = allocator.available();
        framebuffer::write_str("  Kernel heap available: ");
        // Note: Can't easily format numbers in no_std, so we show a simple message
        if available > 0 {
            framebuffer::write_str("OK (");
            if available >= 1024 * 1024 {
                framebuffer::write_str(">1MB");
            } else {
                framebuffer::write_str("<1MB");
            }
            framebuffer::write_str(")\n");
        } else {
            framebuffer::write_str("LOW\n");
        }
    } else {
        framebuffer::write_str("  Allocator: Not initialized\n");
    }
}

/// Command: mouse - Show mouse status
fn cmd_mouse(_args: &[&str]) {
    framebuffer::write_str("Mouse Status:\n");

    let (x, y) = mouse::get_position();
    let (left, middle, right) = mouse::get_buttons();

    framebuffer::write_str("  Position: X=");
    framebuffer::write_str(format_int(x));
    framebuffer::write_str(", Y=");
    framebuffer::write_str(format_int(y));
    framebuffer::write_str("\n");

    framebuffer::write_str("  Buttons: ");
    if left {
        framebuffer::write_str("L");
    }
    if middle {
        framebuffer::write_str("M");
    }
    if right {
        framebuffer::write_str("R");
    }
    if !left && !middle && !right {
        framebuffer::write_str("(none)");
    }
    framebuffer::write_str("\n");
}

/// Simple integer to string conversion (for small numbers)
fn format_int(n: i16) -> &'static str {
    match n {
        -32768..=-1 => {
            // Negative numbers
            let abs = (-n) as u16;
            // For simplicity, just show a limited range
            if abs <= 9 {
                match abs {
                    0 => "-0",
                    1 => "-1",
                    2 => "-2",
                    3 => "-3",
                    4 => "-4",
                    5 => "-5",
                    6 => "-6",
                    7 => "-7",
                    8 => "-8",
                    9 => "-9",
                    _ => "-?",
                }
            } else {
                "-"
            }
        }
        0..=9 => {
            match n {
                0 => "0",
                1 => "1",
                2 => "2",
                3 => "3",
                4 => "4",
                5 => "5",
                6 => "6",
                7 => "7",
                8 => "8",
                9 => "9",
                _ => "?",
            }
        }
        _ => "?",
    }
}