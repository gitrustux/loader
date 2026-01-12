// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Native Console Driver
//!
//! This module provides native console output after ExitBootServices.
//! It supports both framebuffer-based console and serial console fallback.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::ptr::NonNull;

/// Console driver type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleType {
    /// No console available
    None,
    /// Framebuffer console (for amd64/arm64)
    Framebuffer,
    /// Serial console (fallback for all architectures)
    Serial,
}

/// Console error types
#[derive(Debug)]
pub enum ConsoleError {
    /// Console not available
    NotAvailable,
    /// Buffer full
    BufferFull,
    /// I/O error
    IOError,
}

/// Console driver trait
pub trait ConsoleDriver {
    /// Write data to console
    fn write(&mut self, data: &[u8]) -> Result<(), ConsoleError>;

    /// Flush any buffered output
    fn flush(&mut self) -> Result<(), ConsoleError>;

    /// Check if console is available
    fn is_available(&self) -> bool;
}

/// Serial console configuration
#[repr(C)]
pub struct SerialConfig {
    /// Base I/O port (x86) or MMIO address (ARM)
    pub base: usize,
    /// Baud rate divisor
    pub baud_divisor: u16,
    /// Data bits (typically 8)
    pub data_bits: u8,
    /// Stop bits (typically 1)
    pub stop_bits: u8,
    /// Parity (0=none, 1=odd, 2=even)
    pub parity: u8,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            base: 0x3F8,          // COM1 default
            baud_divisor: 12,     // 9600 baud
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        }
    }
}

/// Serial console driver
pub struct SerialConsole {
    config: SerialConfig,
    initialized: AtomicBool,
}

impl SerialConsole {
    /// Create a new serial console
    pub fn new(config: SerialConfig) -> Self {
        Self {
            config,
            initialized: AtomicBool::new(false),
        }
    }

    /// Initialize the serial port
    fn init(&self) {
        if self.initialized.load(Ordering::Acquire) {
            return;
        }

        unsafe {
            let base = self.config.base as u16;

            // Disable interrupts
            self.outb(base + 1, 0x00);

            // Enable DLAB (set baud rate divisor)
            self.outb(base + 3, 0x80);

            // Set divisor (lo byte, hi byte)
            self.outb(base, (self.config.baud_divisor & 0xFF) as u8);
            self.outb(base + 1, (self.config.baud_divisor >> 8) as u8);

            // 8 bits, no parity, one stop bit
            self.outb(base + 3, 0x03);

            // Enable FIFO, clear, 14-byte threshold
            self.outb(base + 2, 0xC7);

            // IRQs enabled, RTS/DSR set
            self.outb(base + 4, 0x0B);
        }

        self.initialized.store(true, Ordering::Release);
    }

    /// Write a byte to serial port
    #[inline(always)]
    unsafe fn outb(&self, port: u16, val: u8) {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") val,
            options(nomem, nostack)
        );

        #[cfg(target_arch = "aarch64")]
        {
            // For ARM, use MMIO
            let ptr = port as *mut u8;
            ptr.write_volatile(val);
        }
    }

    /// Read a byte from serial port
    #[inline(always)]
    unsafe fn inb(&self, port: u16) -> u8 {
        #[cfg(target_arch = "x86_64")]
        {
            let value: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") port,
                out("al") value,
                options(nomem, nostack)
            );
            value
        }

        #[cfg(target_arch = "aarch64")]
        {
            // For ARM, use MMIO
            let ptr = port as *const u8;
            ptr.read_volatile()
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            // Default stub for other architectures
            0
        }
    }

    /// Check if transmit buffer is empty
    fn is_transmit_ready(&self) -> bool {
        unsafe {
            let base = self.config.base as u16;
            (self.inb(base + 5) & 0x20) != 0
        }
    }

    /// Write a single byte to serial port
    fn write_byte(&self, byte: u8) {
        while !self.is_transmit_ready() {
            core::hint::spin_loop();
        }

        unsafe {
            let base = self.config.base as u16;
            self.outb(base, byte);
        }
    }
}

impl ConsoleDriver for SerialConsole {
    fn write(&mut self, data: &[u8]) -> Result<(), ConsoleError> {
        if !self.initialized.load(Ordering::Acquire) {
            self.init();
        }

        for &byte in data {
            self.write_byte(byte);
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), ConsoleError> {
        // Serial console is always flushed immediately
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Framebuffer console configuration
#[repr(C)]
pub struct FramebufferConfig {
    /// Physical address of framebuffer
    pub address: NonNull<u8>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Stride in pixels
    pub stride: u32,
    /// Bytes per pixel
    pub bytes_per_pixel: u32,
    /// Pixel format (RGB, BGR, etc.)
    pub format: PixelFormat,
}

/// Pixel format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// RGB 8-8-8
    RGB,
    /// BGR 8-8-8
    BGR,
    /// Unknown format
    Unknown,
}

impl Default for FramebufferConfig {
    fn default() -> Self {
        Self {
            // Safe non-null address (will be replaced during init)
            address: unsafe { NonNull::new_unchecked(0xB8000 as *mut u8) },
            width: 80,
            height: 25,
            stride: 80,
            bytes_per_pixel: 2,
            format: PixelFormat::Unknown,
        }
    }
}

/// Framebuffer console driver
pub struct FramebufferConsole {
    config: FramebufferConfig,
    initialized: AtomicBool,
    cursor_x: AtomicU32,
    cursor_y: AtomicU32,
}

impl FramebufferConsole {
    /// Create a new framebuffer console
    pub fn new(config: FramebufferConfig) -> Self {
        Self {
            config,
            initialized: AtomicBool::new(false),
            cursor_x: AtomicU32::new(0),
            cursor_y: AtomicU32::new(0),
        }
    }

    /// Initialize the framebuffer console
    fn init(&self) {
        if self.initialized.load(Ordering::Acquire) {
            return;
        }

        // Clear the screen
        self.clear();

        self.initialized.store(true, Ordering::Release);
    }

    /// Clear the screen
    fn clear(&self) {
        unsafe {
            let ptr = self.config.address.as_ptr();
            let size = (self.config.height * self.config.stride * self.config.bytes_per_pixel) as usize;

            // Fill with black
            core::ptr::write_bytes(ptr, 0, size);
        }

        self.cursor_x.store(0, Ordering::Release);
        self.cursor_y.store(0, Ordering::Release);
    }

    /// Write a character to framebuffer
    fn write_char(&self, c: char) {
        match c {
            '\r' => {
                self.cursor_x.store(0, Ordering::Release);
            }
            '\n' => {
                self.cursor_x.store(0, Ordering::Release);
                let y = self.cursor_y.fetch_add(1, Ordering::AcqRel);
                if y >= self.config.height - 1 {
                    self.scroll();
                }
            }
            c if c.is_ascii() => {
                self.draw_char(c as u8);
                let x = self.cursor_x.fetch_add(1, Ordering::AcqRel);
                if x >= self.config.width - 8 {
                    self.cursor_x.store(0, Ordering::Release);
                    let y = self.cursor_y.fetch_add(1, Ordering::AcqRel);
                    if y >= self.config.height - 16 {
                        self.scroll();
                    }
                }
            }
            _ => {}
        }
    }

    /// Draw a character at current cursor position
    fn draw_char(&self, c: u8) {
        // Simple 8x16 font rendering would go here
        // For now, this is a stub
        let _ = c;
    }

    /// Scroll the screen up by one line
    fn scroll(&self) {
        unsafe {
            let ptr = self.config.address.as_ptr();
            let row_size = (self.config.stride * self.config.bytes_per_pixel) as usize;
            let scroll_size = (self.config.height as usize - 16) * row_size;

            // Copy everything up
            core::ptr::copy(
                ptr.add(row_size * 16),
                ptr,
                scroll_size,
            );

            // Clear bottom line
            let bottom_start = ptr.add(scroll_size);
            core::ptr::write_bytes(bottom_start, 0, row_size * 16);
        }

        self.cursor_y.store(self.cursor_y.load(Ordering::Acquire) - 1, Ordering::Release);
    }
}

impl ConsoleDriver for FramebufferConsole {
    fn write(&mut self, data: &[u8]) -> Result<(), ConsoleError> {
        if !self.initialized.load(Ordering::Acquire) {
            self.init();
        }

        for &byte in data {
            self.write_char(byte as char);
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), ConsoleError> {
        // Framebuffer is directly mapped, no flush needed
        Ok(())
    }

    fn is_available(&self) -> bool {
        !self.config.address.as_ptr().is_null()
    }
}

/// Native console manager
pub struct NativeConsole {
    console_type: AtomicU32, // Stores ConsoleType as u32
    framebuffer: Option<FramebufferConsole>,
    serial: Option<SerialConsole>,
}

unsafe impl Send for NativeConsole {}
unsafe impl Sync for NativeConsole {}

impl NativeConsole {
    /// Create a new native console manager
    pub fn new() -> Self {
        Self {
            console_type: AtomicU32::new(ConsoleType::None as u32),
            framebuffer: None,
            serial: None,
        }
    }

    /// Initialize serial console
    pub fn init_serial(&mut self, config: SerialConfig) {
        let console = SerialConsole::new(config);
        self.serial = Some(console);
        self.console_type.store(ConsoleType::Serial as u32, Ordering::Release);
    }

    /// Initialize framebuffer console
    pub fn init_framebuffer(&mut self, config: FramebufferConfig) {
        let console = FramebufferConsole::new(config);
        self.framebuffer = Some(console);
        self.console_type.store(ConsoleType::Framebuffer as u32, Ordering::Release);
    }

    /// Get current console type
    pub fn get_type(&self) -> ConsoleType {
        match self.console_type.load(Ordering::Acquire) {
            0 => ConsoleType::None,
            1 => ConsoleType::Framebuffer,
            2 => ConsoleType::Serial,
            _ => ConsoleType::None,
        }
    }

    /// Write data to the active console
    pub fn write(&mut self, data: &[u8]) -> Result<(), ConsoleError> {
        match self.get_type() {
            ConsoleType::Framebuffer => {
                if let Some(ref mut console) = self.framebuffer {
                    console.write(data)
                } else {
                    Err(ConsoleError::NotAvailable)
                }
            }
            ConsoleType::Serial => {
                if let Some(ref mut console) = self.serial {
                    console.write(data)
                } else {
                    Err(ConsoleError::NotAvailable)
                }
            }
            ConsoleType::None => Err(ConsoleError::NotAvailable),
        }
    }

    /// Flush the active console
    pub fn flush(&mut self) -> Result<(), ConsoleError> {
        match self.get_type() {
            ConsoleType::Framebuffer => {
                if let Some(ref mut console) = self.framebuffer {
                    console.flush()
                } else {
                    Err(ConsoleError::NotAvailable)
                }
            }
            ConsoleType::Serial => {
                if let Some(ref mut console) = self.serial {
                    console.flush()
                } else {
                    Err(ConsoleError::NotAvailable)
                }
            }
            ConsoleType::None => Err(ConsoleError::NotAvailable),
        }
    }

    /// Write a string to the console
    pub fn write_str(&mut self, s: &str) -> Result<(), ConsoleError> {
        self.write(s.as_bytes())
    }
}

impl Default for NativeConsole {
    fn default() -> Self {
        Self::new()
    }
}

/// Global native console instance (mutable for runtime use)
static mut NATIVE_CONSOLE: NativeConsole = NativeConsole {
    console_type: AtomicU32::new(ConsoleType::None as u32),
    framebuffer: None,
    serial: None,
};

/// Initialize native serial console
pub fn init_serial_console(config: SerialConfig) {
    unsafe {
        NATIVE_CONSOLE.init_serial(config);
    }
}

/// Initialize native framebuffer console
pub fn init_framebuffer_console(config: FramebufferConfig) {
    unsafe {
        NATIVE_CONSOLE.init_framebuffer(config);
    }
}

/// Write to native console
pub fn native_write(data: &[u8]) -> Result<(), ConsoleError> {
    unsafe {
        NATIVE_CONSOLE.write(data)
    }
}

/// Write string to native console
pub fn native_write_str(s: &str) -> Result<(), ConsoleError> {
    unsafe {
        NATIVE_CONSOLE.write_str(s)
    }
}

/// Flush native console
pub fn native_flush() -> Result<(), ConsoleError> {
    unsafe {
        NATIVE_CONSOLE.flush()
    }
}

/// Get native console type
pub fn native_console_type() -> ConsoleType {
    unsafe {
        NATIVE_CONSOLE.get_type()
    }
}
