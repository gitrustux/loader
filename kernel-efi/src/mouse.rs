// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! PS/2 Mouse Driver
//!
//! This module provides a PS/2 mouse driver that works
//! after ExitBootServices. It handles mouse packets from the mouse
//! and tracks cursor position.
//!
//! ## Hardware
//! - Data port: 0x60 (shared with keyboard)
//! - Command/status port: 0x64 (shared with keyboard)
//! - IRQ: IRQ12 (interrupt 40)
//!
//! ## Mouse Packet Format (3-byte standard)
//! - Byte 1: [Y_ovf, X_ovf, Y_sign, X_sign, 1, Mb, Rb, Lb]
//! - Byte 2: X movement (signed delta)
//! - Byte 3: Y movement (signed delta)
//!
//! ## Features
//! - Position tracking
//! - Button state tracking (left, right, middle)
//! - Signed movement (scrolling)
//!
//! ## Limitations
//! - No scroll wheel support (requires IntelliMouse extension)
//! - No extra buttons (5-button mice)
//! - No mouse acceleration

/// Mouse data port (shared with keyboard)
const MOUSE_DATA_PORT: u16 = 0x60;

/// Mouse command/status port (shared with keyboard)
const MOUSE_COMMAND_PORT: u16 = 0x64;

/// Mouse packet size (standard 3-byte)
const MOUSE_PACKET_SIZE: usize = 3;

/// Mouse buffer size (fixed, no heap)
const MOUSE_BUFFER_SIZE: usize = 64;

/// Mouse cursor position and state
#[derive(Debug, Clone, Copy)]
pub struct MouseState {
    pub x: i16,
    pub y: i16,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

/// Mouse packet buffer
struct MouseBuffer {
    packets: [u8; MOUSE_BUFFER_SIZE * MOUSE_PACKET_SIZE],
    write_pos: usize,
}

impl MouseBuffer {
    const fn new() -> Self {
        Self {
            packets: [0; MOUSE_BUFFER_SIZE * MOUSE_PACKET_SIZE],
            write_pos: 0,
        }
    }

    /// Write a packet to the buffer
    fn write(&mut self, packet: &[u8; MOUSE_PACKET_SIZE]) {
        if self.write_pos < MOUSE_BUFFER_SIZE {
            let base = self.write_pos * MOUSE_PACKET_SIZE;
            self.packets[base..base + MOUSE_PACKET_SIZE].copy_from_slice(packet);
            self.write_pos += 1;
        }
    }

    /// Get all pending packets
    fn drain(&mut self) -> &[u8] {
        let count = self.write_pos * MOUSE_PACKET_SIZE;
        self.write_pos = 0;
        &self.packets[..count]
    }
}

/// Global mouse state
static mut MOUSE_STATE: MouseState = MouseState {
    x: 0,
    y: 0,
    left_button: false,
    right_button: false,
    middle_button: false,
};

/// Global mouse packet buffer
static mut MOUSE_BUFFER: MouseBuffer = MouseBuffer::new();

/// Current packet being received
static mut CURRENT_PACKET: [u8; MOUSE_PACKET_SIZE] = [0; MOUSE_PACKET_SIZE];
static mut PACKET_BYTE_COUNT: usize = 0;

/// Initialize the PS/2 mouse driver
///
/// This function enables the mouse and prepares it for input.
pub fn init() {
    unsafe {
        // Reset state
        MOUSE_STATE = MouseState {
            x: 0,
            y: 0,
            left_button: false,
            right_button: false,
            middle_button: false,
        };
        MOUSE_BUFFER = MouseBuffer::new();
        CURRENT_PACKET = [0; MOUSE_PACKET_SIZE];
        PACKET_BYTE_COUNT = 0;

        // Send mouse reset command (0xFF)
        write_mouse_command(0xFF);

        // Wait for acknowledgment (0xAA)
        let ack = read_data_port();
        if ack != 0xAA {
            // Mouse reset failed - might not be connected
            return;
        }

        // Wait for device ID (0x00 for standard mouse)
        let _device_id = read_data_port();

        // Enable mouse (0xF4)
        write_mouse_command(0xF4);

        // Wait for acknowledgment (0xFA)
        let _ack = read_data_port();
    }
}

/// Read a byte from the mouse data port
///
/// # Safety
/// This function uses port I/O and must be called with proper synchronization.
unsafe fn read_data_port() -> u8 {
    let mut value: u8;
    core::arch::asm!(
        "in al, dx",
        inlateout("dx") MOUSE_DATA_PORT => _,
        out("al") value,
        options(nomem, nostack)
    );
    value
}

/// Write a command to the mouse
///
/// # Safety
/// This function uses port I/O and must be called with proper synchronization.
unsafe fn write_mouse_command(command: u8) {
    // Send command to mouse
    core::arch::asm!(
        "out dx, al",
        in("al") command,
        in("dx") MOUSE_COMMAND_PORT,
        options(nomem, nostack)
    );

    // Wait for acknowledgment (0xFA from data port)
    let _ack = read_data_port();
}

/// Get the current mouse state
pub fn get_state() -> MouseState {
    unsafe { MOUSE_STATE }
}

/// IRQ12 mouse interrupt handler
///
/// This function should be called from the IDT interrupt handler for IRQ12.
/// It reads the mouse packet and updates the mouse state.
#[no_mangle]
pub extern "C" fn mouse_irq_handler() {
    unsafe {
        // Read byte from data port
        let byte = read_data_port();

        // Add to current packet
        CURRENT_PACKET[PACKET_BYTE_COUNT] = byte;
        PACKET_BYTE_COUNT += 1;

        // Check if we have a complete packet
        if PACKET_BYTE_COUNT == MOUSE_PACKET_SIZE {
            PACKET_BYTE_COUNT = 0;

            // Parse the packet
            let flags = CURRENT_PACKET[0];
            let x_delta = CURRENT_PACKET[1] as i8;
            let y_delta = CURRENT_PACKET[2] as i8;

            // Check for overflow
            if flags & 0x40 == 0 && flags & 0x80 == 0 {
                // No X overflow
                MOUSE_STATE.x += x_delta as i16;

                // Clamp to valid range
                MOUSE_STATE.x = MOUSE_STATE.x.max(-32767).min(32767);
            }

            if flags & 0x10 == 0 && flags & 0x20 == 0 {
                // No Y overflow (note: Y is inverted for mouse movement)
                MOUSE_STATE.y -= y_delta as i16;

                // Clamp to valid range
                MOUSE_STATE.y = MOUSE_STATE.y.max(-32767).min(32767);
            }

            // Update button states
            MOUSE_STATE.left_button = (flags & 0x01) != 0;
            MOUSE_STATE.right_button = (flags & 0x02) != 0;
            MOUSE_STATE.middle_button = (flags & 0x04) != 0;

            // Store packet for later processing
            MOUSE_BUFFER.write(&CURRENT_PACKET);
        }
    }
}

/// Get all pending mouse packets since last check
///
/// Returns a slice of bytes containing all pending mouse packets.
/// Each packet is 3 bytes, so the returned slice length will be
/// a multiple of 3.
pub fn drain_packets() -> &'static [u8] {
    unsafe {
        MOUSE_BUFFER.drain()
    }
}

/// Check if mouse has moved since last check
///
/// Compares current state with previous state to detect movement.
pub fn has_moved() -> bool {
    unsafe {
        // Simple check: if we have any pending packets, mouse moved
        MOUSE_BUFFER.write_pos > 0
    }
}

/// Get mouse position as (x, y) coordinates
///
/// Returns the current mouse cursor position relative to the center
/// of the framebuffer.
pub fn get_position() -> (i16, i16) {
    let state = unsafe { MOUSE_STATE };
    (state.x, state.y)
}

/// Get button states
///
/// Returns (left, middle, right) button states as booleans.
pub fn get_buttons() -> (bool, bool, bool) {
    let state = unsafe { MOUSE_STATE };
    (state.left_button, state.middle_button, state.right_button)
}
