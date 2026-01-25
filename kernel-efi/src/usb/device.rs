// Copyright 2025 The Rustux Authors
//
// USB Device Enumeration and HID Keyboard
//
// This module handles USB device enumeration for HID keyboard detection.

use crate::usb::{UsbError, trb::*};

/// USB HID Boot Protocol keyboard report size
pub const HID_REPORT_SIZE: usize = 8;

/// USB control request types
pub const USB_REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const USB_REQ_SET_ADDRESS: u8 = 0x05;
pub const USB_REQ_SET_CONFIGURATION: u8 = 0x09;
pub const USB_REQ_SET_INTERFACE: u8 = 0x0B;
pub const USB_REQ_SET_PROTOCOL: u8 = 0x0B;

/// USB descriptor types
pub const USB_DT_DEVICE: u8 = 0x01;
pub const USB_DT_CONFIG: u8 = 0x02;
pub const USB_DT_STRING: u8 = 0x03;
pub const USB_DT_INTERFACE: u8 = 0x04;
pub const USB_DT_ENDPOINT: u8 = 0x05;
pub const USB_DT_HID: u8 = 0x21;
pub const USB_DT_REPORT: u8 = 0x22;

/// USB HID protocol
pub const USB_HID_BOOT_PROTOCOL: u8 = 0x00;
pub const USB_HID_REPORT_PROTOCOL: u8 = 0x01;

/// USB HID class codes
pub const USB_CLASS_HID: u8 = 0x03;
pub const USB_HID_SUBCLASS_BOOT: u8 = 0x01;
pub const USB_HID_PROTOCOL_KEYBOARD: u8 = 0x01;

/// USB endpoint types
pub const USB_ENDPOINT_XFER_CONTROL: u8 = 0x00;
pub const USB_ENDPOINT_XFER_ISOC: u8 = 0x01;
pub const USB_ENDPOINT_XFER_BULK: u8 = 0x02;
pub const USB_ENDPOINT_XFER_INT: u8 = 0x03;

/// USB endpoint direction
pub const USB_DIR_OUT: u8 = 0x00;
pub const USB_DIR_IN: u8 = 0x80;

/// Transfer poll timeout (iterations)
const TRANSFER_TIMEOUT: u32 = 100000;

/// USB device information for HID keyboard
#[derive(Debug, Clone, Copy)]
pub struct HidKeyboardInfo {
    pub slot_id: u8,
    pub endpoint_addr: u8,
    pub max_packet: u16,
    pub interval: u8,
}

/// Global HID keyboard device info
static mut HID_KEYBOARD: Option<HidKeyboardInfo> = None;

/// HID keyboard data buffer
static mut HID_REPORT_BUFFER: [u8; HID_REPORT_SIZE] = [0; HID_REPORT_SIZE];

/// Active transfer flag (to prevent multiple concurrent transfers)
static mut TRANSFER_ACTIVE: bool = false;

/// Check if we have a valid HID keyboard device
pub fn has_hid_keyboard() -> bool {
    unsafe { HID_KEYBOARD.is_some() }
}

/// Get HID keyboard device info
pub fn get_hid_keyboard() -> Option<HidKeyboardInfo> {
    unsafe { HID_KEYBOARD }
}

/// Simple USB HID keyboard enumeration
///
/// For Phase 7A, this is a simplified implementation that:
/// 1. Checks for a device on USB ports 1-4
/// 2. Resets the port
/// 3. Assumes it's a HID keyboard (for QEMU testing)
///
/// Phase 7B will implement full USB enumeration.
pub unsafe fn enumerate_hid_keyboard() -> Result<HidKeyboardInfo, UsbError> {
    use crate::usb::xhci;

    // Get xHCI controller
    let controller = xhci::controller().ok_or(UsbError::XhciNotFound)?;

    // Check ports 1-4 for a connected device
    let mut found_port = None;
    for port in 1..=4 {
        if controller.check_port_connection(port) {
            found_port = Some(port);
            break;
        }
    }

    let port = found_port.ok_or(UsbError::DeviceNotFound)?;

    // Reset the port
    controller.reset_port(port)?;

    // For QEMU testing, assume:
    // - Slot ID 1 (first device)
    // - Endpoint 1 (interrupt IN)
    // - 8-byte max packet (HID boot keyboard)
    // - 10ms polling interval

    // TODO: Phase 7B - Implement full USB enumeration:
    // - Enable slot (Enable Slot command TRB)
    // - Read device descriptor (GET_DESCRIPTOR control transfer)
    // - Set device address (SET_ADDRESS control transfer)
    // - Read configuration descriptor
    // - Find HID interface with boot protocol
    // - Set boot protocol (SET_PROTOCOL control transfer)
    // - Find interrupt IN endpoint
    // - Configure endpoint

    let keyboard_info = HidKeyboardInfo {
        slot_id: 1,
        endpoint_addr: 0x81,  // Endpoint 1, IN direction
        max_packet: 8,
        interval: 10,
    };

    HID_KEYBOARD = Some(keyboard_info);

    Ok(keyboard_info)
}

/// Read HID keyboard report via interrupt transfer
///
/// This function performs an xHCI interrupt IN transfer to read keyboard data.
/// Returns the number of bytes read (0-8).
pub unsafe fn read_keyboard_report() -> usize {
    use crate::usb::xhci;

    // Check if transfer is already active (prevent concurrent transfers)
    if TRANSFER_ACTIVE {
        // Check if previous transfer completed
        if let Some(event) = xhci::controller().and_then(|c| c.poll_events()) {
            TRANSFER_ACTIVE = false;

            // Check completion code
            let code = completion_code(event.status);
            if code != CompletionCode::Success {
                return 0;
            }

            // Get transfer length from event (bits 0-23)
            let length = (event.status & 0x00FF_FFFF) as usize;
            return length;
        }
        return 0;
    }

    // Check if we have a HID keyboard
    let keyboard = match get_hid_keyboard() {
        Some(k) => k,
        None => return 0,
    };

    // Get controller
    let controller = match xhci::controller() {
        Some(c) => c,
        None => return 0,
    };

    // Get controller mut for transfer ring access
    let controller_mut = match xhci::controller_mut() {
        Some(c) => c,
        None => return 0,
    };

    // Step 1: Create Normal TRB for interrupt IN transfer
    let data_ptr = get_report_buffer_ptr();

    // Extract endpoint number from address (lower 4 bits)
    let endpoint_num = (keyboard.endpoint_addr & 0x0F) as u8;

    // TRB: Normal Interrupt IN
    // - data_ptr: physical address of HID_REPORT_BUFFER
    // - status: TRB length (8 bytes) + TD size + interrupter target
    // - control: TRB Type (1) + Direction (IN) + IOC + Cycle bit (handled by enqueue)
    let trb = NormalTrb {
        data_ptr,
        status: (HID_REPORT_SIZE as u32) & 0x00FFFFFF,  // TRB Length
        control: (1 << 10) |  // TRB Type = Normal (1)
                  (1 << 16) |  // Interrupt On Complete (IOC)
                  (1 << 5),    // ISP (Interrupt on Short Packet)
    };

    // Step 2: Enqueue TRB to transfer ring
    if let Err(_) = controller_mut.transfer_ring.enqueue(&trb) {
        return 0;  // Ring full
    }

    // Step 3: Ring doorbell for endpoint 1
    // Note: We use endpoint 1 (DCI = 2, where DCI = (ep_num * 2) + direction)
    // For interrupt IN endpoint 1, DCI = (1 * 2) + 1 = 3
    let dci = (endpoint_num as u8) * 2 + 1;
    controller.ring_doorbell(keyboard.slot_id, dci, 0);

    // Set transfer active flag
    TRANSFER_ACTIVE = true;

    // Step 4: Poll for event completion
    let mut timeout = TRANSFER_TIMEOUT;
    while timeout > 0 {
        if let Some(event) = controller.poll_events() {
            TRANSFER_ACTIVE = false;

            // Check completion code
            let code = completion_code(event.status);
            if code != CompletionCode::Success {
                return 0;
            }

            // Get transfer length from event (bits 0-23)
            let length = (event.status & 0x00FF_FFFF) as usize;
            return length.min(HID_REPORT_SIZE);
        }

        timeout -= 1;
        for _ in 0..10 {
            core::arch::asm!("nop", options(nomem, nostack));
        }
    }

    // Timeout - transfer still pending, will be checked next call
    0
}

/// Get the HID report buffer address (for TRB data pointer)
pub fn get_report_buffer_ptr() -> u64 {
    unsafe { &HID_REPORT_BUFFER as *const u8 as u64 }
}

/// Parse the HID report buffer into a KeyboardReport
pub unsafe fn parse_report_buffer() -> Option<super::hid::KeyboardReport> {
    let ptr = &HID_REPORT_BUFFER as *const u8 as *const super::hid::KeyboardReport;
    Some(*ptr)
}

/// USB setup packet structure
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct UsbSetupPacket {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

/// Create a GET_DESCRIPTOR setup packet
pub fn get_descriptor_setup_packet(desc_type: u8, desc_index: u8, length: u16) -> UsbSetupPacket {
    UsbSetupPacket {
        bm_request_type: 0x80,  // Device-to-host, Type=Standard, Recipient=Device
        b_request: USB_REQ_GET_DESCRIPTOR,
        w_value: ((desc_type as u16) << 8) | (desc_index as u16),
        w_index: 0,
        w_length: length,
    }
}

/// Create a SET_ADDRESS setup packet
pub fn set_address_setup_packet(address: u8) -> UsbSetupPacket {
    UsbSetupPacket {
        bm_request_type: 0x00,  // Host-to-device, Type=Standard, Recipient=Device
        b_request: USB_REQ_SET_ADDRESS,
        w_value: address as u16,
        w_index: 0,
        w_length: 0,
    }
}

/// Create a SET_CONFIGURATION setup packet
pub fn set_configuration_setup_packet(config: u8) -> UsbSetupPacket {
    UsbSetupPacket {
        bm_request_type: 0x00,  // Host-to-device, Type=Standard, Recipient=Device
        b_request: USB_REQ_SET_CONFIGURATION,
        w_value: config as u16,
        w_index: 0,
        w_length: 0,
    }
}

/// Create a SET_PROTOCOL (HID boot protocol) setup packet
pub fn set_boot_protocol_setup_packet() -> UsbSetupPacket {
    UsbSetupPacket {
        bm_request_type: 0x21,  // Host-to-device, Type=Class, Recipient=Interface
        b_request: USB_REQ_SET_PROTOCOL,
        w_value: USB_HID_BOOT_PROTOCOL as u16,
        w_index: 0,  // Interface 0
        w_length: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_packet_size() {
        assert_eq!(core::mem::size_of::<UsbSetupPacket>(), 8);
    }

    #[test]
    fn test_get_descriptor_setup() {
        let setup = get_descriptor_setup_packet(USB_DT_DEVICE, 0, 18);
        assert_eq!(setup.bm_request_type, 0x80);
        assert_eq!(setup.b_request, USB_REQ_GET_DESCRIPTOR);
        assert_eq!(setup.w_value, 0x0100);  // Type=1, Index=0
        assert_eq!(setup.w_length, 18);
    }

    #[test]
    fn test_set_address_setup() {
        let setup = set_address_setup_packet(5);
        assert_eq!(setup.bm_request_type, 0x00);
        assert_eq!(setup.b_request, USB_REQ_SET_ADDRESS);
        assert_eq!(setup.w_value, 5);
        assert_eq!(setup.w_length, 0);
    }

    #[test]
    fn test_hid_report_size() {
        assert_eq!(HID_REPORT_SIZE, 8);
    }
}
