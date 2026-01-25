// Copyright 2025 The Rustux Authors
//
// USB Stack - Minimal Phase 7A Implementation
//
// This module provides USB HID keyboard support using polling.
// No IRQs, no scheduler dependency - works immediately after boot.

pub mod pci;
pub mod xhci;
pub mod device;
pub mod hid;
pub mod trb;

/// USB error type
#[derive(Debug, Clone, Copy)]
pub enum UsbError {
    /// XHCI controller not found
    XhciNotFound,
    /// XHCI initialization failed
    XhciInitFailed,
    /// USB device not found
    DeviceNotFound,
    /// HID keyboard not found
    HidKeyboardNotFound,
    /// Timeout
    Timeout,
}

/// XHCI controller information
#[derive(Debug, Clone, Copy)]
pub struct XhciInfo {
    pub mmio_base: u64,
}
