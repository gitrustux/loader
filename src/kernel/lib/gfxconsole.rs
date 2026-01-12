// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Graphics Console
//!
//! This module provides a text console on a graphics surface.
//! It handles terminal emulation, scrolling, and character display.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use spin::Mutex;

use crate::rustux::types::*;
use crate::kernel::lib::gfx::{GfxFont, GfxSurface};
use crate::kernel::lib::vga_font::VGA_FONT;

/// Default text color (white)
pub const TEXT_COLOR: u32 = 0xFFFFFFFF;

/// Default background color (black)
pub const BACK_COLOR: u32 = 0xFF000000;

/// Crash text color (white)
pub const CRASH_TEXT_COLOR: u32 = 0xFFFFFFFF;

/// Crash background color (red)
pub const CRASH_BACK_COLOR: u32 = 0xFFE000E0;

/// Console state
#[derive(Debug)]
pub enum ConsoleState {
    Normal,
    Escape,
}

/// Graphics console state
pub struct GfxConsole {
    /// Main surface to draw on
    pub surface: Option<*mut GfxSurface>,
    /// Underlying hardware surface
    pub hw_surface: Option<*mut GfxSurface>,
    /// Font to use
    pub font: Option<&'static GfxFont>,
    /// Number of rows
    pub rows: u32,
    /// Number of columns
    pub columns: u32,
    /// Extra pixels left over
    pub extray: u32,
    /// Current cursor X position
    pub x: AtomicU32,
    /// Current cursor Y position
    pub y: AtomicU32,
    /// Front color
    pub front_color: AtomicU32,
    /// Back color
    pub back_color: AtomicU32,
    /// Console state
    pub state: Mutex<ConsoleState>,
    /// Escape sequence parameter
    pub escape_param: Mutex<u32>,
}

unsafe impl Send for GfxConsole {}
unsafe impl Sync for GfxConsole {}

impl GfxConsole {
    /// Create a new graphics console
    ///
    /// # Returns
    ///
    /// New console instance
    pub fn new() -> Self {
        Self {
            surface: None,
            hw_surface: None,
            font: None,
            rows: 0,
            columns: 0,
            extray: 0,
            x: AtomicU32::new(0),
            y: AtomicU32::new(0),
            front_color: AtomicU32::new(TEXT_COLOR),
            back_color: AtomicU32::new(BACK_COLOR),
            state: Mutex::new(ConsoleState::Normal),
            escape_param: Mutex::new(0),
        }
    }

    /// Setup the console with surfaces
    ///
    /// # Arguments
    ///
    /// * `surface` - Software surface
    /// * `hw_surface` - Hardware surface (may be same as surface)
    /// * `font` - Font to use
    pub fn setup(
        &mut self,
        surface: *mut GfxSurface,
        hw_surface: *mut GfxSurface,
        font: &'static GfxFont,
    ) {
        self.surface = Some(surface);
        self.hw_surface = Some(hw_surface);
        self.font = Some(font);

        // Calculate rows and columns
        if let Some(s) = unsafe { surface.as_ref() } {
            self.rows = s.height / font.height;
            self.columns = s.width / font.width;
            self.extray = s.height - (self.rows * font.height);
        }

        println!(
            "GfxConsole: {} rows, {} columns, {} extra pixels",
            self.rows, self.columns, self.extray
        );
    }

    /// Clear the console
    ///
    /// # Arguments
    ///
    /// * `crash_console` - Whether this is a crash console
    pub fn clear(&self, crash_console: bool) {
        // Reset cursor position
        self.x.store(0, Ordering::Release);
        self.y.store(0, Ordering::Release);

        // Set colors
        if crash_console {
            self.front_color.store(CRASH_TEXT_COLOR, Ordering::Release);
            self.back_color.store(CRASH_BACK_COLOR, Ordering::Release);
        } else {
            self.front_color.store(TEXT_COLOR, Ordering::Release);
            self.back_color.store(BACK_COLOR, Ordering::Release);
        }

        // Fill screen with background color
        if let Some(surface) = self.surface {
            unsafe {
                if let Some(s) = surface.as_mut() {
                    let back = self.back_color.load(Ordering::Acquire);
                    s.fillrect(0, 0, s.width, s.height, back);
                }
            }
        }
    }

    /// Put a character on the console
    ///
    /// # Arguments
    ///
    /// * `c` - Character to display
    ///
    /// # Returns
    ///
    /// true if screen should be refreshed
    pub fn putc(&self, c: char) -> bool {
        let mut state = self.state.lock();
        let mut escape_param = self.escape_param.lock();
        let mut needs_refresh = false;

        match *state {
            ConsoleState::Normal => match c {
                '\r' => {
                    self.x.store(0, Ordering::Release);
                }
                '\n' => {
                    let y = self.y.fetch_add(1, Ordering::AcqRel);
                    if y + 1 >= self.rows {
                        needs_refresh = true;
                    }
                }
                '\x08' => {
                    // Backspace
                    let x = self.x.load(Ordering::Acquire);
                    if x > 0 {
                        self.x.store(x - 1, Ordering::Release);
                    }
                }
                '\t' => {
                    // Tab
                    let x = self.x.load(Ordering::Acquire);
                    self.x.store((x + 8) & !7, Ordering::Release);
                }
                '\x1b' => {
                    // Escape
                    *escape_param = 0;
                    *state = ConsoleState::Escape;
                }
                _ => {
                    self.draw_char(c as u8);
                    let x = self.x.fetch_add(1, Ordering::AcqRel);
                    if x + 1 >= self.columns {
                        self.x.store(0, Ordering::Release);
                        let y = self.y.fetch_add(1, Ordering::AcqRel);
                        if y + 1 >= self.rows {
                            needs_refresh = true;
                        }
                    }
                }
            },
            ConsoleState::Escape => {
                if c.is_ascii_digit() {
                    *escape_param = *escape_param * 10 + (c as u32 - '0' as u32);
                } else if c == 'D' {
                    let p = *escape_param;
                    let x = self.x.load(Ordering::Acquire);
                    if p <= x {
                        self.x.store(x - p, Ordering::Release);
                    }
                    *state = ConsoleState::Normal;
                } else if c == '[' {
                    // Eat this character
                } else {
                    self.draw_char(c as u8);
                    let x = self.x.fetch_add(1, Ordering::AcqRel);
                    if x + 1 >= self.columns {
                        self.x.store(0, Ordering::Release);
                        let y = self.y.fetch_add(1, Ordering::AcqRel);
                        if y + 1 >= self.rows {
                            needs_refresh = true;
                        }
                    }
                    *state = ConsoleState::Normal;
                }
            }
        }

        // Handle scrolling
        let y = self.y.load(Ordering::Acquire);
        if y >= self.rows {
            self.scroll_up();
        }

        needs_refresh
    }

    /// Draw a character at current cursor position
    fn draw_char(&self, c: u8) {
        if let (Some(surface), Some(font)) = (self.surface, self.font) {
            let x = self.x.load(Ordering::Acquire);
            let y = self.y.load(Ordering::Acquire);
            let front = self.front_color.load(Ordering::Acquire);
            let back = self.back_color.load(Ordering::Acquire);

            let pixel_x = x * font.width;
            let pixel_y = y * font.height;

            unsafe {
                if let Some(s) = surface.as_mut() {
                    crate::kernel::lib::gfx::gfx_putchar(
                        s,
                        font,
                        c,
                        pixel_x,
                        pixel_y,
                        front,
                        back,
                    );
                }
            }
        }
    }

    /// Scroll the console up by one line
    fn scroll_up(&self) {
        if let (Some(surface), Some(font)) = (self.surface, self.font) {
            let char_height = font.height;
            unsafe {
                if let Some(s) = surface.as_mut() {
                    // Copy all lines up by one character row
                    s.copyrect(
                        0,
                        char_height,
                        s.width,
                        s.height - char_height,
                        0,
                        0,
                    );

                    // Clear the new bottom line
                    let back = self.back_color.load(Ordering::Acquire);
                    s.fillrect(
                        0,
                        s.height - char_height,
                        s.width,
                        char_height,
                        back,
                    );
                }
            }
        }
    }

    /// Clear a line
    ///
    /// # Arguments
    ///
    /// * `y` - Line number to clear
    fn clear_line(&self, y: u32) {
        if let (Some(surface), Some(font)) = (self.surface, self.font) {
            unsafe {
                if let Some(s) = surface.as_mut() {
                    let back = self.back_color.load(Ordering::Acquire);
                    s.fillrect(
                        0,
                        y * font.height,
                        s.width,
                        font.height,
                        back,
                    );
                }
            }
        }
    }

    /// Flush the console to hardware
    pub fn flush(&self) {
        // Flush software surface to display
        if let Some(surface) = self.surface {
            unsafe {
                if let Some(s) = surface.as_ref() {
                    s.flush();
                }
            }
        }
    }

    /// Put a pixel directly (bypasses console)
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `color` - Color value
    pub fn putpixel(&self, x: u32, y: u32, color: u32) {
        if let Some(surface) = self.surface {
            unsafe {
                if let Some(s) = surface.as_mut() {
                    s.putpixel(x, y, color);
                }
            }
        }
    }
}

impl Default for GfxConsole {
    fn default() -> Self {
        Self::new()
    }
}

/// Global graphics console instance
static GFX_CONSOLE: Mutex<Option<GfxConsole>> = Mutex::new(None);

/// Print callback for graphics console
///
/// # Arguments
///
/// * `str` - String to print
/// * `len` - Length of string
pub fn gfxconsole_print_callback(str: &str, len: usize) {
    let console = GFX_CONSOLE.lock();

    if let Some(ref c) = *console {
        let mut needs_refresh = false;

        for (i, &byte) in str.as_bytes().iter().enumerate() {
            if i < len {
                // Handle LF -> CRLF
                if byte == b'\n' {
                    needs_refresh |= c.putc('\r');
                }
                needs_refresh |= c.putc(byte as char);
            }
        }

        if needs_refresh {
            c.flush();
        }
    }
}

/// Start graphics console
///
/// # Arguments
///
/// * `surface` - Software surface
/// * `hw_surface` - Hardware surface
pub fn gfxconsole_start(surface: *mut GfxSurface, hw_surface: *mut GfxSurface) {
    let mut console = GFX_CONSOLE.lock();

    if console.is_some() {
        println!("GfxConsole: Already started");
        return;
    }

    // TODO: Get font from config
    // For now, use a placeholder
    let font = placeholder_font();

    let mut new_console = GfxConsole::new();
    new_console.setup(surface, hw_surface, font);
    new_console.clear(false);

    *console = Some(new_console);

    println!("GfxConsole: Started");
}

/// Placeholder font (TODO: load actual font)
fn placeholder_font() -> &'static GfxFont {
    // Use the built-in VGA 8x16 font
    &VGA_FONT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_creation() {
        let console = GfxConsole::new();
        assert_eq!(console.rows, 0);
        assert_eq!(console.columns, 0);
    }

    #[test]
    fn test_colors() {
        assert_eq!(TEXT_COLOR, 0xFFFFFFFF);
        assert_eq!(BACK_COLOR, 0xFF000000);
        assert_eq!(CRASH_TEXT_COLOR, 0xFFFFFFFF);
        assert_eq!(CRASH_BACK_COLOR, 0xFFE000E0);
    }

    #[test]
    fn test_putc_basic() {
        let console = GfxConsole::new();
        // Basic test - just ensure it doesn't panic
        console.putc('A');
        assert_eq!(console.x.load(Ordering::Acquire), 1);
    }
}
