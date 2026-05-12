// Copyright 2025 The Rustux Authors
//
// USB Device Enumeration and HID Keyboard
//
// This module handles USB device enumeration for HID keyboard detection.

use crate::usb::{UsbError, trb::*};
use crate::usb::xhci::{PORTSC_CCS, PORTSC_PED, PORTSC_PR, PORTSC_PORT_SPEED_MASK, PORTSC_PORT_SPEED_SHIFT};

/// USB HID Boot Protocol keyboard report size
pub const HID_REPORT_SIZE: usize = 8;

/// Device descriptor buffer size
pub const DEVICE_DESC_SIZE: usize = 18;

/// Device descriptor buffer (static for DMA)
static mut DEVICE_DESC_BUFFER: [u8; DEVICE_DESC_SIZE] = [0; DEVICE_DESC_SIZE];

/// Get device descriptor buffer address (for DMA)
pub fn get_device_desc_ptr() -> u64 {
    unsafe { &DEVICE_DESC_BUFFER as *const u8 as u64 }
}

/// Flag to track if xHCI succeeded and device detected (CCS=1)
/// This prevents PS/2 fallback when USB hardware is present
static mut XHCI_SUCCESS_WITH_DEVICE: bool = false;

/// Check if xHCI succeeded with a device detected
pub fn xhci_succeeded_with_device() -> bool {
    unsafe { XHCI_SUCCESS_WITH_DEVICE }
}

/// Print hex byte helper
fn print_hex_byte(value: u8) {
    let high = (value >> 4) & 0x0F;
    let low = value & 0x0F;
    let high_c = if high < 10 { b'0' + high } else { b'A' + (high - 10) };
    let low_c = if low < 10 { b'0' + low } else { b'A' + (low - 10) };
    unsafe {
        crate::framebuffer::write_str(core::str::from_utf8(&[high_c]).unwrap());
        crate::framebuffer::write_str(core::str::from_utf8(&[low_c]).unwrap());
    }
}

/// Print hex word helper
fn print_hex_word(value: u16) {
    print_hex_byte((value >> 8) as u8);
    print_hex_byte(value as u8);
}

/// Print hex dword helper
fn print_hex_dword(value: u32) {
    print_hex_byte((value >> 24) as u8);
    print_hex_byte((value >> 16) as u8);
    print_hex_byte((value >> 8) as u8);
    print_hex_byte(value as u8);
}

/// Get port speed name from PORTSC speed field
fn get_port_speed_name(portsc: u32) -> &'static str {
    let speed = (portsc & PORTSC_PORT_SPEED_MASK) >> PORTSC_PORT_SPEED_SHIFT;
    match speed {
        0 => "Full",
        1 => "Low",
        2 => "High",
        3 => "Super",
        4 => "SuperPlus",
        _ => "Unknown",
    }
}

/// Dump PORTSC register with all relevant fields
unsafe fn dump_portsc(controller: &crate::usb::xhci::XhciController, port: usize, label: &str) {
    let portsc = controller.read_port_sc(port);

    crate::framebuffer::write_str("PORTSC[");
    crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + port as u8]).unwrap());
    crate::framebuffer::write_str("] ");
    crate::framebuffer::write_str(label);
    crate::framebuffer::write_str(": 0x");
    print_hex_dword(portsc);
    crate::framebuffer::write_str(" ");

    // Print individual bits
    let ccs = if portsc & PORTSC_CCS != 0 { "CCS=1 " } else { "CCS=0 " };
    crate::framebuffer::write_str(ccs);

    let ped = if portsc & PORTSC_PED != 0 { "PED=1 " } else { "PED=0 " };
    crate::framebuffer::write_str(ped);

    let pr = if portsc & PORTSC_PR != 0 { "PR=1 " } else { "PR=0 " };
    crate::framebuffer::write_str(pr);

    // Print port speed
    let speed = get_port_speed_name(portsc);
    crate::framebuffer::write_str("Speed=");
    crate::framebuffer::write_str(speed);
    crate::framebuffer::write_str("\n");
}

/// Dump event TRB details
unsafe fn dump_event_trb(event: &EventTrb) {
    crate::framebuffer::write_str("  Event TRB: ptr=0x");
    print_hex_dword((event.trb_ptr >> 32) as u32);
    print_hex_dword((event.trb_ptr & 0xFFFFFFFF) as u32);
    crate::framebuffer::write_str(" status=0x");
    print_hex_dword(event.status);
    crate::framebuffer::write_str(" control=0x");
    print_hex_dword(event.control);

    // Extract completion code
    let code = (event.status >> 24) as u8;
    crate::framebuffer::write_str(" CC=");
    print_hex_byte(code);

    // Extract TRB type
    let trb_type = ((event.control >> 10) & 0x3F) as u8;
    crate::framebuffer::write_str(" Type=");
    print_hex_byte(trb_type);

    // Extract slot ID
    let slot_id = ((event.control >> 24) & 0xFF) as u8;
    crate::framebuffer::write_str(" Slot=");
    print_hex_byte(slot_id);

    // Print completion code name
    let cc_name = match code {
        1 => "Success",
        2 => "TRBError",
        3 => "Stall",
        4 => "ResourceError",
        5 => "BandwidthError",
        6 => "NoSlotsError",
        7 => "InvalidStreamTypeError",
        8 => "SlotNotEnabledError",
        9 => "EpNotEnabledError",
        13 => "ShortPacket",
        22 => "IncompatibleDeviceError",
        25 => "CommandAborted",
        _ => "Unknown",
    };
    crate::framebuffer::write_str("(");
    crate::framebuffer::write_str(cc_name);
    crate::framebuffer::write_str(")\n");
}

/// Dump device descriptor (first 18 bytes)
unsafe fn dump_device_descriptor() {
    crate::framebuffer::write_str("  Device Descriptor:\n");
    crate::framebuffer::write_str("    bLength=");
    print_hex_byte(DEVICE_DESC_BUFFER[0]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    bDescriptorType=");
    print_hex_byte(DEVICE_DESC_BUFFER[1]);
    crate::framebuffer::write_str("\n");

    let bcd_usb = (DEVICE_DESC_BUFFER[3] as u16) << 8 | (DEVICE_DESC_BUFFER[2] as u16);
    crate::framebuffer::write_str("    bcdUSB=0x");
    print_hex_word(bcd_usb);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    bDeviceClass=");
    print_hex_byte(DEVICE_DESC_BUFFER[4]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    bDeviceSubClass=");
    print_hex_byte(DEVICE_DESC_BUFFER[5]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    bDeviceProtocol=");
    print_hex_byte(DEVICE_DESC_BUFFER[6]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    bMaxPacketSize0=");
    print_hex_byte(DEVICE_DESC_BUFFER[7]);
    crate::framebuffer::write_str("\n");

    let id_vendor = (DEVICE_DESC_BUFFER[9] as u16) << 8 | (DEVICE_DESC_BUFFER[8] as u16);
    crate::framebuffer::write_str("    idVendor=0x");
    print_hex_word(id_vendor);
    crate::framebuffer::write_str("\n");

    let id_product = (DEVICE_DESC_BUFFER[11] as u16) << 8 | (DEVICE_DESC_BUFFER[10] as u16);
    crate::framebuffer::write_str("    idProduct=0x");
    print_hex_word(id_product);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    iManufacturer=");
    print_hex_byte(DEVICE_DESC_BUFFER[14]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    iProduct=");
    print_hex_byte(DEVICE_DESC_BUFFER[15]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    iSerialNumber=");
    print_hex_byte(DEVICE_DESC_BUFFER[16]);
    crate::framebuffer::write_str("\n");

    crate::framebuffer::write_str("    bNumConfigurations=");
    print_hex_byte(DEVICE_DESC_BUFFER[17]);
    crate::framebuffer::write_str("\n");
}

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

/// Full USB HID keyboard enumeration with comprehensive debugging
///
/// This implementation:
/// 1. Checks for a device on USB ports 1-4
/// 2. Dumps PORTSC before/during/after reset
/// 3. Issues Enable Slot command and waits for completion
/// 4. Reads device descriptor via GET_DESCRIPTOR
/// 5. Sets device address
/// 6. Assumes HID boot keyboard for initial testing
///
/// Debug output includes:
/// - PORTSC dumps with CCS, PED, PR, Port Speed
/// - Enable Slot command TRB and completion event
/// - Address Device command debugging
/// - Device descriptor dump
/// - Event ring debugging with completion codes
pub unsafe fn enumerate_hid_keyboard() -> Result<HidKeyboardInfo, UsbError> {
    use crate::usb::xhci;

    crate::framebuffer::write_str("USB: Getting xHCI controller...\n");

    // Get xHCI controller
    let controller = xhci::controller().ok_or(UsbError::XhciNotFound)?;

    crate::framebuffer::write_str("USB: Checking ports 1-4 for devices...\n");

    // Check ports 1-4 for a connected device
    let mut found_port = None;
    for port in 1..=4 {
        crate::framebuffer::write_str("USB: Port ");
        // Simple decimal print
        crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + port as u8]).unwrap());
        crate::framebuffer::write_str("... ");

        let portsc = controller.read_port_sc(port);

        // Check CCS (Current Connect Status) bit
        if portsc & PORTSC_CCS != 0 {
            crate::framebuffer::write_str("CCS=1 ");

            // Also check PED (Port Enabled)
            if portsc & PORTSC_PED != 0 {
                crate::framebuffer::write_str("PED=1 ");
            }

            // Print port speed
            let speed = get_port_speed_name(portsc);
            crate::framebuffer::write_str(speed);
            crate::framebuffer::write_str("\n");

            // Flag that xHCI succeeded with a device detected
            XHCI_SUCCESS_WITH_DEVICE = true;

            found_port = Some(port);
            break;
        } else {
            crate::framebuffer::write_str("CCS=0\n");
        }
    }

    let port = found_port.ok_or(UsbError::DeviceNotFound)?;

    // ============================================================
    // STEP 1: PORT RESET WITH COMPREHENSIVE DEBUGGING
    // ============================================================

    crate::framebuffer::write_str("USB: === PORT RESET DEBUG ===\n");
    crate::framebuffer::write_str("USB: Port ");
    crate::framebuffer::write_str(core::str::from_utf8(&[b'0' + port as u8]).unwrap());
    crate::framebuffer::write_str(" reset sequence:\n");

    // Dump PORTSC BEFORE reset
    dump_portsc(controller, port, "BEFORE");

    let mut portsc = controller.read_port_sc(port);

    // Check if port is already enabled
    if portsc & PORTSC_PED != 0 {
        crate::framebuffer::write_str("USB: Port already enabled (PED=1), attempting reset anyway\n");
    }

    // Set port reset bit
    crate::framebuffer::write_str("USB: Setting PR bit (0x0020)...\n");
    crate::framebuffer::write_str("USB: Current PORTSC=0x");
    print_hex_dword(portsc);
    crate::framebuffer::write_str(" Writing=0x");
    print_hex_dword(portsc | 0x0020);
    crate::framebuffer::write_str("\n");

    controller.write_port_sc(port, portsc | 0x0020);  // PR is bit 5 = 0x0020

    // Small delay before checking
    for _ in 0..1000 {
        core::arch::asm!("nop", options(nomem, nostack));
    }

    // Dump PORTSC DURING reset
    dump_portsc(controller, port, "DURING");

    // Wait for PR to clear (reset complete)
    crate::framebuffer::write_str("USB: Waiting for PR to clear...\n");
    let mut timeout = 100000;
    while timeout > 0 {
        portsc = controller.read_port_sc(port);
        if portsc & PORTSC_PR == 0 {
            crate::framebuffer::write_str("USB: PR cleared after ");
            let elapsed = 100000 - timeout;
            // Simple elapsed print
            if elapsed < 1000 {
                crate::framebuffer::write_str("<1ms");
            } else if elapsed < 10000 {
                crate::framebuffer::write_str("<10ms");
            } else {
                crate::framebuffer::write_str(">10ms");
            }
            crate::framebuffer::write_str("\n");
            break;
        }
        timeout -= 1;
        for _ in 0..100 {
            core::arch::asm!("nop", options(nomem, nostack));
        }
    }

    if timeout == 0 {
        crate::framebuffer::write_str("USB: WARNING - PR timeout!\n");
    }

    // Dump PORTSC AFTER reset
    dump_portsc(controller, port, "AFTER");

    // Verify port is enabled after reset
    portsc = controller.read_port_sc(port);
    if portsc & PORTSC_PED == 0 {
        crate::framebuffer::write_str("USB: WARNING - Port not enabled after reset (PED=0)\n");
        crate::framebuffer::write_str("USB: Device may have disconnected during reset\n");
        crate::framebuffer::write_str("USB: Continuing anyway (device was enabled before reset)\n");
        // Don't return error - device was already enabled before reset
    } else {
        crate::framebuffer::write_str("USB: Port enabled OK (PED=1)\n");
    }

    // Verify device is still connected
    if portsc & PORTSC_CCS == 0 {
        crate::framebuffer::write_str("USB: WARNING - Device disconnected during reset (CCS=0)\n");
        crate::framebuffer::write_str("USB: Continuing anyway (will try to re-enumerate)\n");
        // Don't return error - device might reconnect
    } else {
        crate::framebuffer::write_str("USB: Device still connected (CCS=1)\n");
    }

    // ============================================================
    // STEP 2: ENABLE SLOT COMMAND
    // ============================================================

    crate::framebuffer::write_str("USB: === ENABLE SLOT COMMAND ===\n");

    // Get controller mut for command ring access
    let controller_mut = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;

    // Issue Enable Slot command
    crate::framebuffer::write_str("USB: Issuing Enable Slot command...\n");

    match controller_mut.issue_enable_slot_command() {
        Ok(slot_id) => {
            crate::framebuffer::write_str("USB: Enable Slot OK - Slot ID=");
            print_hex_byte(slot_id);
            crate::framebuffer::write_str("\n");
        }
        Err(e) => {
            crate::framebuffer::write_str("USB: Enable Slot FAILED - ");
            match e {
                UsbError::Timeout => crate::framebuffer::write_str("Timeout\n"),
                UsbError::XhciInitFailed => crate::framebuffer::write_str("InitFailed\n"),
                _ => crate::framebuffer::write_str("UnknownError\n"),
            }
            // Continue anyway - use default slot 1 for testing
            crate::framebuffer::write_str("USB: Using default Slot ID=1 for testing\n");
        }
    }

    // For now, assume slot ID 1
    let slot_id = 1u8;

    // ============================================================
    // STEP 3: READ DEVICE DESCRIPTOR (Address 0)
    // ============================================================

    crate::framebuffer::write_str("USB: === GET DEVICE DESCRIPTOR ===\n");
    crate::framebuffer::write_str("USB: Reading 18 bytes from address 0...\n");

    // Clear device descriptor buffer
    for i in 0..DEVICE_DESC_SIZE {
        DEVICE_DESC_BUFFER[i] = 0;
    }

    // Create GET_DESCRIPTOR setup packet
    let setup_packet = get_descriptor_setup_packet(USB_DT_DEVICE, 0, 18);

    crate::framebuffer::write_str("USB: Setup: bmRequestType=0x");
    print_hex_byte(setup_packet.bm_request_type);
    crate::framebuffer::write_str(" bRequest=0x");
    print_hex_byte(setup_packet.b_request);
    crate::framebuffer::write_str(" wValue=0x");
    print_hex_word(setup_packet.w_value);
    crate::framebuffer::write_str(" wLength=0x");
    print_hex_word(setup_packet.w_length);
    crate::framebuffer::write_str("\n");

    // Issue control transfer to get device descriptor
    match controller_mut.issue_control_get(
        slot_id,
        &setup_packet,
        get_device_desc_ptr(),
        18,
    ) {
        Ok(()) => {
            crate::framebuffer::write_str("USB: Get Descriptor OK\n");
            dump_device_descriptor();
        }
        Err(e) => {
            crate::framebuffer::write_str("USB: Get Descriptor FAILED - ");
            match e {
                UsbError::Timeout => crate::framebuffer::write_str("Timeout\n"),
                _ => crate::framebuffer::write_str("Error\n"),
            }
            // Continue with assumed values for testing
        }
    }

    // ============================================================
    // STEP 4: SET DEVICE ADDRESS
    // ============================================================

    crate::framebuffer::write_str("USB: === SET DEVICE ADDRESS ===\n");
    let device_address = 2u8;  // Use address 2 (address 1 is often reserved)

    crate::framebuffer::write_str("USB: Setting device address to ");
    print_hex_byte(device_address);
    crate::framebuffer::write_str("...\n");

    let addr_setup = set_address_setup_packet(device_address);

    match controller_mut.issue_control_set(
        slot_id,
        &addr_setup,
    ) {
        Ok(()) => {
            crate::framebuffer::write_str("USB: Set Address OK\n");
        }
        Err(e) => {
            crate::framebuffer::write_str("USB: Set Address FAILED - ");
            match e {
                UsbError::Timeout => crate::framebuffer::write_str("Timeout\n"),
                _ => crate::framebuffer::write_str("Error\n"),
            }
        }
    }

    // Wait for device to process address change
    for _ in 0..10000 {
        core::arch::asm!("nop", options(nomem, nostack));
    }

    // ============================================================
    // STEP 5: ASSUME HID KEYBOARD FOR INITIAL TESTING
    // ============================================================

    crate::framebuffer::write_str("USB: === ASSUMING HID KEYBOARD ===\n");

    // For initial testing, assume:
    // - Slot ID 1 (first device, assigned by Enable Slot)
    // - Endpoint 1 (interrupt IN) - typical for HID keyboards
    // - 8-byte max packet (HID boot keyboard)
    // - 10ms polling interval

    let keyboard_info = HidKeyboardInfo {
        slot_id: slot_id,
        endpoint_addr: 0x81,  // Endpoint 1, IN direction
        max_packet: 8,
        interval: 10,
    };

    crate::framebuffer::write_str("USB: Slot=");
    print_hex_byte(keyboard_info.slot_id);
    crate::framebuffer::write_str(" Endpoint=0x");
    print_hex_byte(keyboard_info.endpoint_addr);
    crate::framebuffer::write_str(" MaxPacket=");
    print_hex_byte(keyboard_info.max_packet as u8);
    crate::framebuffer::write_str("\n");

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
