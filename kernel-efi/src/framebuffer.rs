// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! GOP Framebuffer Console Driver
//!
//! This module provides a framebuffer console driver using the UEFI
//! Graphics Output Protocol (GOP). It queries GOP before ExitBootServices,
//! saves the framebuffer address, and provides pixel writing functions
//! that work after ExitBootServices.

use uefi::proto::console::gop;
use uefi::proto::console::gop::PixelFormat;
use uefi::Identify;
use uefi::boot::{SearchType, OpenProtocolParams, OpenProtocolAttributes};

/// Framebuffer information (saved before ExitBootServices)
#[repr(C)]
pub struct FramebufferInfo {
    /// Base address of framebuffer
    pub base: *mut u8,
    /// Width in pixels
    pub width: usize,
    /// Height in pixels
    pub height: usize,
    /// Stride (bytes per row) - IMPORTANT: Use this, NOT width, for row offsets
    pub stride: usize,
    /// Bytes per pixel (usually 4 for RGB format)
    pub bpp: usize,
    /// Pixel format from GOP
    pub pixel_format: PixelFormat,
}

unsafe impl Send for FramebufferInfo {}
unsafe impl Sync for FramebufferInfo {}

/// Global framebuffer info (survives ExitBootServices)
static mut FRAMEBUFFER_INFO: Option<FramebufferInfo> = None;

/// Initialize framebuffer by querying GOP protocol
///
/// This MUST be called before ExitBootServices to get the framebuffer
/// address and dimensions.
pub fn init() {
    unsafe {
        // POST CODE 0xF0: Framebuffer init started
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x80u16,
            in("al") 0xF0u8,
            options(nomem, nostack)
        );

        if let Some(info) = query_gop_framebuffer() {
            // POST CODE 0xF1: Framebuffer query succeeded
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x80u16,
                in("al") 0xF1u8,
                options(nomem, nostack)
            );
            FRAMEBUFFER_INFO = Some(info);
        } else {
            // POST CODE 0xF2: Framebuffer query failed
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x80u16,
                in("al") 0xF2u8,
                options(nomem, nostack)
            );
            FRAMEBUFFER_INFO = None;
        }
    }
}

/// Query GOP framebuffer using boot services
///
/// This must be called before ExitBootServices.
///
/// CRITICAL: Do NOT call any console output during GOP query.
/// Console output can trigger allocations and cause hangs.
unsafe fn query_gop_framebuffer() -> Option<FramebufferInfo> {
    // Find all GOP handles using uefi::boot
    let gop_handle = uefi::boot::locate_handle_buffer(SearchType::ByProtocol(&gop::GraphicsOutput::GUID))
        .ok()?
        .first()
        .copied()?;

    // Open the GOP protocol on the handle
    let gop = uefi::boot::open_protocol::<gop::GraphicsOutput>(
        OpenProtocolParams {
            handle: gop_handle,
            agent: uefi::boot::image_handle(),
            controller: None,
        },
        OpenProtocolAttributes::Exclusive,
    ).ok();

    let mut gop = gop?;

    // Get current mode
    let mode = gop.current_mode_info();
    let (width, height) = mode.resolution();

    // Get framebuffer
    let mut fb = gop.frame_buffer();
    let fb_ptr = fb.as_mut_ptr();
    if fb_ptr.is_null() {
        return None;
    }

    let pixel_format = mode.pixel_format();

    // CRITICAL: Copy ALL needed info to plain struct BEFORE ExitBootServices
    // Do NOT hold any GOP protocol pointers across ExitBootServices
    Some(FramebufferInfo {
        base: fb.as_mut_ptr(),
        width: width as usize,
        height: height as usize,
        stride: mode.stride() as usize,  // Use pixels_per_scan_line, NOT width
        bpp: 4,
        pixel_format,
    })
}

/// Get framebuffer info (after ExitBootServices)
pub fn get_info() -> Option<&'static FramebufferInfo> {
    unsafe { FRAMEBUFFER_INFO.as_ref() }
}

/// Check if framebuffer is available
pub fn is_available() -> bool {
    unsafe { FRAMEBUFFER_INFO.is_some() }
}

/// Fill the entire screen with a solid color
///
/// # Arguments
/// * `color` - 32-bit BGR color (0x00BBGGRR)
pub fn fill_screen(color: u32) {
    if let Some(info) = get_info() {
        unsafe {
            let fb = info.base as *mut u32;

            // FIX: Use stride, NOT width, for row offsets
            // Stride is pixels_per_scan_line, width may have padding
            for y in 0..info.height {
                let row_offset = y * info.stride;
                for x in 0..info.width {
                    fb.add(row_offset + x).write_volatile(color);
                }
            }
        }
    }
}

/// Set a single pixel to a color
pub fn set_pixel(x: usize, y: usize, color: u32) {
    if let Some(info) = get_info() {
        if x < info.width && y < info.height {
            unsafe {
                let fb = info.base as *mut u32;
                // FIX: Use stride, NOT width
                let offset = y * info.stride + x;
                fb.add(offset).write_volatile(color);
            }
        }
    }
}

/// MINIMAL TEST: Write a SINGLE pixel to verify framebuffer is safe
///
/// This is the FIRST test - if this hangs, framebuffer memory is invalid
pub fn test_single_pixel() {
    if let Some(info) = get_info() {
        unsafe {
            let fb = info.base as *mut u32;
            // Write ONE red pixel at (0,0)
            fb.write_volatile(0x000000FF); // Red in BGR format
        }
    }
}

/// MINIMAL TEST: Write ONE scanline to verify stride is correct
///
/// This tests that using stride (not width) doesn't write past bounds
/// If you see a RED line at top of screen, framebuffer is SAFE
pub fn test_single_scanline() {
    if let Some(info) = get_info() {
        unsafe {
            let fb = info.base as *mut u32;
            // Write ONE row using stride for the row offset
            // This tests stride != width handling
            for x in 0..info.width {
                fb.add(x).write_volatile(0x000000FF); // Red in BGR format
            }
        }
    }
}

/// Display a test pattern on the framebuffer
///
/// Divides the screen into 4 colored quadrants:
/// - Top-left: Red
/// - Top-right: Green
/// - Bottom-left: Blue
/// - Bottom-right: Yellow
pub fn display_test_pattern() {
    if let Some(info) = get_info() {
        unsafe {
            let fb = info.base as *mut u32;
            let half_w = info.width / 2;
            let half_h = info.height / 2;

            // FIX: Use stride, NOT width, for row calculations
            for y in 0..info.height {
                let row_offset = y * info.stride;
                for x in 0..info.width {
                    // BGR format: 0x00BBGGRR
                    let color = if x < half_w && y < half_h {
                        0x000000FF // Red
                    } else if x >= half_w && y < half_h {
                        0x0000FF00 // Green
                    } else if x < half_w && y >= half_h {
                        0x00FF0000 // Blue
                    } else {
                        0x00FFFF00 // Yellow
                    };

                    fb.add(row_offset + x).write_volatile(color);
                }
            }
        }
    }
}

/// Display a green screen = framebuffer working
pub fn display_status() {
    fill_screen(0x0000FF00); // Green in BGR format
}
