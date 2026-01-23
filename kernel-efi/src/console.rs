// Copyright 2025 The Rustux Authors
// SPDX-License-Identifier: MIT

//! Console abstraction with explicit boot/runtime separation.
//!
//! BOOT MODE:
//!   - Uses UEFI stdout ONLY
//!
//! RUNTIME MODE:
//!   - Uses framebuffer text renderer ONLY
//!   - NO UEFI calls are permitted
//!
//! This file enforces UEFI correctness.

use core::sync::atomic::{AtomicU8, Ordering};

use uefi::{CStr16, cstr16};

use crate::framebuffer;

/// Console operating mode
#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq)]
enum ConsoleMode {
    BootUefi = 0,
    RuntimeFramebuffer = 1,
}

/// Global console mode (boot by default)
static CONSOLE_MODE: AtomicU8 = AtomicU8::new(ConsoleMode::BootUefi as u8);

#[inline(always)]
fn mode() -> ConsoleMode {
    match CONSOLE_MODE.load(Ordering::SeqCst) {
        1 => ConsoleMode::RuntimeFramebuffer,
        _ => ConsoleMode::BootUefi,
    }
}

/// Initialize console (BOOT ONLY)
///
/// Must be called before ExitBootServices.
pub fn init_boot_console() {
    // Boot console uses UEFI stdout directly
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("[CONSOLE] Boot console initialized\r\n"));
    });
}

/// Switch console to framebuffer mode
///
/// MUST be called immediately BEFORE ExitBootServices.
pub fn switch_to_framebuffer_console() {
    CONSOLE_MODE.store(ConsoleMode::RuntimeFramebuffer as u8, Ordering::SeqCst);
}

/// Write a string to the active console
///
/// Safe to call in both boot and runtime.
pub fn write_str(s: &str) {
    match mode() {
        ConsoleMode::BootUefi => write_uefi(s),
        ConsoleMode::RuntimeFramebuffer => framebuffer::write_str(s),
    }
}

/// Write a line (adds newline)
pub fn write_line(s: &str) {
    write_str(s);
    write_str("\n");
}

/// BOOT-ONLY output via UEFI stdout
#[inline(always)]
fn write_uefi(s: &str) {
    // Convert &str → UTF-16 buffer (stack only)
    let mut buf: [u16; 256] = [0; 256];
    let mut i = 0;

    for c in s.encode_utf16() {
        if i >= buf.len() - 1 {
            break;
        }
        buf[i] = c;
        i += 1;
    }
    buf[i] = 0;

    let Ok(cstr) = CStr16::from_u16_with_nul(&buf[..=i]) else {
        return;
    };

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(&cstr);
    });
}

/// Panic-safe console output (last resort)
pub fn emergency_write(s: &str) {
    match mode() {
        ConsoleMode::BootUefi => write_uefi(s),
        ConsoleMode::RuntimeFramebuffer => framebuffer::emergency_write(s),
    }
}
