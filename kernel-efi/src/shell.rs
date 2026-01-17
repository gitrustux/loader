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
        name: "irq",
        description: "Show keyboard IRQ debug info",
        handler: cmd_irq,
    },
    Command {
        name: "kbd",
        description: "Test keyboard input (non-blocking)",
        handler: cmd_kbd,
    },
    Command {
        name: "pic",
        description: "Show PIC configuration",
        handler: cmd_pic,
    },
    Command {
        name: "flush",
        description: "Flush keyboard buffer (fixes stale keys)",
        handler: cmd_flush,
    },
];

/// Run the Rustux CLI shell
///
/// This function never returns. It continuously:
/// 1. Prints the prompt with Dracula theme colors
/// 2. Reads a line from the keyboard
/// 3. Parses and executes the command
/// 4. Repeats
pub fn run_shell() -> ! {
    // Dracula theme colors
    use framebuffer::colors;

    // Print shell header with Dracula theme
    framebuffer::write_str_color("\n\n*** RUSTUX SHELL v0.1 ***", colors::encode(colors::GREEN));
    framebuffer::write_str("\n");
    framebuffer::write_str_color("Type 'help' for available commands.", colors::encode(colors::COMMENT));
    framebuffer::write_str("\n\n");

    let mut buffer = [0u8; COMMAND_BUFFER_SIZE];

    loop {
        // Print Dracula-themed prompt: "> rustux> "
        // Using ASCII-only characters for VGA font compatibility
        // Arrow: Green (>)
        framebuffer::write_str_color("> ", colors::encode(colors::GREEN));
        // Username: Cyan (rustux)
        framebuffer::write_str_color("rustux", colors::encode(colors::CYAN));
        // Chevron: Purple (>)
        framebuffer::write_str_color("> ", colors::encode(colors::PURPLE));

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
            framebuffer::write_str_color("Unknown command: ", colors::encode(colors::RED));
            framebuffer::write_str(command_name);
            framebuffer::write_str("\n");
            framebuffer::write_str_color("Type 'help' for available commands.", colors::encode(colors::COMMENT));
            framebuffer::write_str("\n");
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

/// Command: irq - Show keyboard IRQ debug info
fn cmd_irq(_args: &[&str]) {
    framebuffer::write_str("Keyboard IRQ Debug Info:\n");

    let irq_count = keyboard::get_irq_count();
    framebuffer::write_str("  IRQ count: ");
    if irq_count > 0 {
        framebuffer::write_str("(IRQs firing!)\n");
        // Show the actual count (0-15)
        let digit = if irq_count <= 9 {
            (b'0' + irq_count) as char
        } else {
            (b'A' + irq_count - 10) as char
        };
        framebuffer::write_str("  Count: ");
        framebuffer::write_str_color(
            unsafe { core::str::from_utf8_unchecked(&[digit as u8]) },
            framebuffer::colors::encode(framebuffer::colors::GREEN)
        );
        framebuffer::write_str("\n");
    } else {
        framebuffer::write_str_color("0 (IRQ NOT firing!)\n", framebuffer::colors::encode(framebuffer::colors::RED));
    }

    // Try direct keyboard read
    if let Some(c) = keyboard::try_read_char_direct() {
        framebuffer::write_str("  Hardware: Key detected (");
        framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&[c as u8]) });
        framebuffer::write_str(")\n");
    } else {
        framebuffer::write_str("  Hardware: No key pressed\n");
    }
}

/// Command: kbd - Test keyboard input (non-blocking)
fn cmd_kbd(_args: &[&str]) {
    framebuffer::write_str("Testing keyboard (5 seconds, press any key)...\n");

    let mut buffer = [0u8; 32];
    let mut received = 0;

    // Wait for input with timeout
    for _ in 0..50000 {
        if let Some(c) = keyboard::read_char() {
            // Got input via IRQ
            framebuffer::write_str_color("IRQ: ", framebuffer::colors::encode(framebuffer::colors::GREEN));
            framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&[c as u8]) });
            framebuffer::write_str("\n");
            received = 1;
            break;
        } else if let Some(c) = keyboard::try_read_char_direct() {
            // Got input via direct polling
            framebuffer::write_str_color("POLL: ", framebuffer::colors::encode(framebuffer::colors::YELLOW));
            framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&[c as u8]) });
            framebuffer::write_str("\n");
            received = 2;
            break;
        }

        for _ in 0..100 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)); }
        }
    }

    if received == 0 {
        framebuffer::write_str_color("TIMEOUT - No input received\n", framebuffer::colors::encode(framebuffer::colors::RED));
    } else if received == 1 {
        framebuffer::write_str_color("SUCCESS - IRQ driver working!\n", framebuffer::colors::encode(framebuffer::colors::GREEN));
    } else {
        framebuffer::write_str_color("IRQ NOT working - polling works\n", framebuffer::colors::encode(framebuffer::colors::YELLOW));
    }
}

/// Command: pic - Show PIC configuration
fn cmd_pic(_args: &[&str]) {
    use crate::runtime;

    framebuffer::write_str("PIC Configuration:\n");

    unsafe {
        let (pic1_mask, pic2_mask) = runtime::pic_get_masks();
        let (pic1_irr, pic2_irr) = runtime::pic_get_irr();
        let (pic1_isr, pic2_isr) = runtime::pic_get_isr();

        // PIC1 (IRQs 0-7)
        framebuffer::write_str("  PIC1 (IRQ 0-7):\n");
        framebuffer::write_str("    Mask: ");
        cmd_pic_print_bits(pic1_mask);
        framebuffer::write_str(if pic1_mask & 0x02 == 0 { " (IRQ1 enabled)\n" } else { " (IRQ1 DISABLED!)\n" });

        framebuffer::write_str("    IRR:  ");
        cmd_pic_print_bits(pic1_irr);
        framebuffer::write_str(if pic1_irr & 0x02 != 0 { " (IRQ1 pending)\n" } else { " (no IRQ1)\n" });

        framebuffer::write_str("    ISR:  ");
        cmd_pic_print_bits(pic1_isr);
        framebuffer::write_str(if pic1_isr & 0x02 != 0 { " (IRQ1 in-service)\n" } else { " (no IRQ1 active)\n" });

        // PIC2 (IRQs 8-15)
        framebuffer::write_str("  PIC2 (IRQ 8-15):\n");
        framebuffer::write_str("    Mask: ");
        cmd_pic_print_bits(pic2_mask);
        framebuffer::write_str("\n");

        framebuffer::write_str("    IRR:  ");
        cmd_pic_print_bits(pic2_irr);
        framebuffer::write_str("\n");

        framebuffer::write_str("    ISR:  ");
        cmd_pic_print_bits(pic2_isr);
        framebuffer::write_str("\n");
    }
}

/// Helper: Print 8 bits as binary
fn cmd_pic_print_bits(value: u8) {
    for i in (0..8).rev() {
        let bit = if value & (1 << i) != 0 { b'1' } else { b'0' };
        framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&[bit]) });
    }
    framebuffer::write_str(" (0x");
    // Show hex
    let hex_hi = (value >> 4) & 0x0F;
    let hex_lo = value & 0x0F;
    let h = if hex_hi < 10 { b'0' + hex_hi } else { b'A' + hex_hi - 10 };
    let l = if hex_lo < 10 { b'0' + hex_lo } else { b'A' + hex_lo - 10 };
    framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&[h, l]) });
    framebuffer::write_str(")");
}

/// Command: flush - Flush keyboard buffer
fn cmd_flush(_args: &[&str]) {
    framebuffer::write_str("Flushing keyboard buffer...\n");

    // Call the keyboard flush function
    keyboard::flush();

    framebuffer::write_str_color("Keyboard buffer flushed!\n", framebuffer::colors::encode(framebuffer::colors::GREEN));
    framebuffer::write_str("If keys still show wrong characters, IRQ1 may not be firing.\n");
    framebuffer::write_str("Try typing to see if input works now.\n");
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