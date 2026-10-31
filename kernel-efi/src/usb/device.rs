// Copyright 2025 The Rustux Authors
//
// USB Device Enumeration and HID Keyboard
//
// This module handles USB device enumeration for HID keyboard detection.

use crate::usb::{UsbError, trb::*};
use crate::usb::xhci::{
    PORTSC_CCS, PORTSC_PED, PORTSC_PR, PORTSC_PORT_SPEED_MASK, PORTSC_PORT_SPEED_SHIFT,
    SPEED_FULL, SPEED_LOW, SPEED_HIGH, SPEED_SUPER,
};
use crate::usb::dma::{
    DCBAA_DATA, INPUT_CONTEXT,
    input_context_base, device_context_base,
    ep0_ring_base, ep1_in_ring_base,
    ep1_in_ring_enqueue, reset_device_state,
};

/// USB HID Boot Protocol keyboard report size
pub const HID_REPORT_SIZE: usize = 8;

/// Device descriptor buffer size
pub const DEVICE_DESC_SIZE: usize = 18;

/// Device descriptor buffer (static for DMA)
static mut DEVICE_DESC_BUFFER: [u8; DEVICE_DESC_SIZE] = [0; DEVICE_DESC_SIZE];

/// Configuration descriptor buffer (static for DMA)
const CONFIG_DESC_SIZE: usize = 256;
static mut CONFIG_DESC_BUFFER: [u8; CONFIG_DESC_SIZE] = [0; CONFIG_DESC_SIZE];

/// Get device descriptor buffer address (for DMA)
pub fn get_device_desc_ptr() -> u64 {
    unsafe { &DEVICE_DESC_BUFFER as *const u8 as u64 }
}

/// Set when a USB boot keyboard interface was identified, even if a later
/// enumeration step failed. Other USB devices (webcam, Bluetooth, ...) do not
/// set it, so PS/2 fallback stays available on laptops.
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
        1 => "Full",
        2 => "Low",
        3 => "High",
        4 => "Super",
        5 => "SuperPlus",
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
pub const USB_REQ_SET_IDLE: u8 = 0x0A;

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

/// Find a USB HID boot keyboard on any root hub port.
///
/// Every connected port is tried in turn. For each device: reset the port
/// (USB2 only), Enable Slot, Address Device, read the device and
/// configuration descriptors, and look for a HID boot keyboard interface
/// (class 3 / subclass 1 / protocol 1). Devices that are not keyboards are
/// released (Disable Slot) and the next port is tried.
pub unsafe fn enumerate_hid_keyboard() -> Result<HidKeyboardInfo, UsbError> {
    use crate::usb::xhci;

    let controller = xhci::controller().ok_or(UsbError::XhciNotFound)?;
    let max_ports = controller.max_ports();

    crate::framebuffer::write_str("USB: Scanning ");
    print_decimal(max_ports as u32);
    crate::framebuffer::write_str(" ports\n");

    let mut any_device = false;
    for port in 1..=max_ports {
        let controller = xhci::controller().ok_or(UsbError::XhciNotFound)?;
        let portsc = controller.read_port_sc(port);
        if portsc & PORTSC_CCS == 0 {
            continue;
        }
        any_device = true;

        crate::framebuffer::write_str("USB: Port ");
        print_decimal(port as u32);
        crate::framebuffer::write_str(": ");
        crate::framebuffer::write_str(get_port_speed_name(portsc));
        crate::framebuffer::write_str("-speed device\n");

        match try_port(port) {
            Ok(info) => {
                HID_KEYBOARD = Some(info);
                return Ok(info);
            }
            Err(_) => {
                // try_port has already printed the reason and released the slot
                reset_device_state();
            }
        }
    }

    if any_device {
        Err(UsbError::HidKeyboardNotFound)
    } else {
        Err(UsbError::DeviceNotFound)
    }
}

/// Enumerate the device on one port. Releases the slot on failure.
unsafe fn try_port(port: usize) -> Result<HidKeyboardInfo, UsbError> {
    use crate::usb::xhci;

    reset_device_state();

    // USB2 ports need a port reset to become enabled; USB3 ports enable
    // themselves after link training.
    let controller = xhci::controller().ok_or(UsbError::XhciNotFound)?;
    let portsc = controller.read_port_sc(port);
    let speed = (portsc & PORTSC_PORT_SPEED_MASK) >> PORTSC_PORT_SPEED_SHIFT;
    if speed < SPEED_SUPER || portsc & PORTSC_PED == 0 {
        if let Err(e) = controller.reset_port(port) {
            crate::framebuffer::write_str("  port reset failed\n");
            return Err(e);
        }
    }
    // Speed is only valid after reset for USB2
    let portsc = controller.read_port_sc(port);
    let speed = (portsc & PORTSC_PORT_SPEED_MASK) >> PORTSC_PORT_SPEED_SHIFT;

    let controller_mut = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    let slot_id = controller_mut.issue_enable_slot_command()?;

    match configure_device(port, speed, slot_id) {
        Ok(info) => Ok(info),
        Err(e) => {
            let controller_mut = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
            let _ = controller_mut.issue_disable_slot_command(slot_id);
            DCBAA_DATA.data[slot_id as usize] = 0;
            Err(e)
        }
    }
}

/// Default EP0 max packet size for a port speed
fn default_ep0_mps(speed: u32) -> u32 {
    match speed {
        SPEED_LOW | SPEED_FULL => 8,
        SPEED_HIGH => 64,
        _ => 512,
    }
}

/// xHCI Interval field for an interrupt endpoint (xHCI 6.2.3.6)
fn interrupt_interval(speed: u32, b_interval: u8) -> u32 {
    let b = core::cmp::max(b_interval as u32, 1);
    match speed {
        SPEED_LOW | SPEED_FULL => {
            // bInterval is in 1 ms frames; Interval is 2^n * 125 us
            let mut n = 3;
            while n < 10 && (1u32 << (n + 1)) <= b * 8 {
                n += 1;
            }
            n
        }
        _ => core::cmp::min(b - 1, 15),
    }
}

unsafe fn configure_device(port: usize, speed: u32, slot_id: u8) -> Result<HidKeyboardInfo, UsbError> {
    use crate::usb::xhci;

    // ---------------- Address Device ----------------
    DCBAA_DATA.data[slot_id as usize] = device_context_base();

    let mut ep0_mps = default_ep0_mps(speed);
    INPUT_CONTEXT.control.drop_flags = 0;
    INPUT_CONTEXT.control.add_flags = 0x3; // Slot + EP0
    INPUT_CONTEXT.slot.dword0 = (1u32 << 27) | (speed << 20); // Context Entries=1, Speed
    INPUT_CONTEXT.slot.dword1 = (port as u32) << 16; // Root Hub Port Number (1-based)
    INPUT_CONTEXT.slot.dword2 = 0;
    INPUT_CONTEXT.slot.dword3 = 0;

    let ep0_ring = ep0_ring_base();
    INPUT_CONTEXT.ep[0].dword0 = 0;
    INPUT_CONTEXT.ep[0].dword1 = (3u32 << 1) | (4u32 << 3) | (ep0_mps << 16); // CErr=3, Control
    INPUT_CONTEXT.ep[0].deq_ptr_lo = (ep0_ring as u32) | 1; // DCS=1
    INPUT_CONTEXT.ep[0].deq_ptr_hi = (ep0_ring >> 32) as u32;
    INPUT_CONTEXT.ep[0].dword4 = 8; // Average TRB Length

    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_address_device_command(slot_id, input_context_base(), false)?;
    spin(2_000_000); // SET_ADDRESS recovery (2 ms)

    // ---------------- Device descriptor ----------------
    for b in DEVICE_DESC_BUFFER.iter_mut() {
        *b = 0;
    }
    // Read the first 8 bytes to learn bMaxPacketSize0
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_control_get(slot_id, &get_descriptor_setup_packet(USB_DT_DEVICE, 0, 8), get_device_desc_ptr(), 8)?;
    let mps0 = DEVICE_DESC_BUFFER[7] as u32;
    if speed < SPEED_SUPER && mps0 != 0 && mps0 != ep0_mps {
        // Update EP0 max packet size with Evaluate Context
        ep0_mps = mps0;
        INPUT_CONTEXT.control.add_flags = 0x2; // EP0 only
        INPUT_CONTEXT.ep[0].dword1 = (3u32 << 1) | (4u32 << 3) | (ep0_mps << 16);
        let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
        c.issue_evaluate_context_command(slot_id, input_context_base())?;
    }
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_control_get(slot_id, &get_descriptor_setup_packet(USB_DT_DEVICE, 0, 18), get_device_desc_ptr(), 18)?;

    let vid = (DEVICE_DESC_BUFFER[9] as u16) << 8 | DEVICE_DESC_BUFFER[8] as u16;
    let pid = (DEVICE_DESC_BUFFER[11] as u16) << 8 | DEVICE_DESC_BUFFER[10] as u16;
    crate::framebuffer::write_str("  device ");
    print_hex_word(vid);
    crate::framebuffer::write_str(":");
    print_hex_word(pid);
    crate::framebuffer::write_str(" class=");
    print_hex_byte(DEVICE_DESC_BUFFER[4]);
    crate::framebuffer::write_str("\n");

    // ---------------- Configuration descriptor ----------------
    for b in CONFIG_DESC_BUFFER.iter_mut() {
        *b = 0;
    }
    let cfg_ptr = &CONFIG_DESC_BUFFER as *const u8 as u64;
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_control_get(slot_id, &get_descriptor_setup_packet(USB_DT_CONFIG, 0, 9), cfg_ptr, 9)?;
    let total = core::cmp::min(
        (CONFIG_DESC_BUFFER[3] as usize) << 8 | CONFIG_DESC_BUFFER[2] as usize,
        CONFIG_DESC_SIZE,
    );
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    let got = c.issue_control_get(slot_id, &get_descriptor_setup_packet(USB_DT_CONFIG, 0, total as u16), cfg_ptr, total as u16)? as usize;
    let config_value = CONFIG_DESC_BUFFER[5];

    let kbd = match find_boot_keyboard(&CONFIG_DESC_BUFFER[..core::cmp::min(got, total)]) {
        Some(k) => k,
        None => {
            crate::framebuffer::write_str("  not a boot keyboard, skipping\n");
            return Err(UsbError::HidKeyboardNotFound);
        }
    };
    XHCI_SUCCESS_WITH_DEVICE = true;
    crate::framebuffer::write_str("  boot keyboard: interface ");
    print_decimal(kbd.interface as u32);
    crate::framebuffer::write_str(", endpoint 0x");
    print_hex_byte(kbd.endpoint_addr);
    crate::framebuffer::write_str("\n");

    // ---------------- SET_CONFIGURATION / SET_PROTOCOL / SET_IDLE ----------------
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_control_set(slot_id, &set_configuration_setup_packet(config_value))?;

    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    if c.issue_control_set(slot_id, &set_boot_protocol_setup_packet(kbd.interface)).is_err() {
        crate::framebuffer::write_str("  SET_PROTOCOL failed (continuing)\n");
    }
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    let _ = c.issue_control_set(slot_id, &set_idle_setup_packet(kbd.interface)); // optional

    // ---------------- Configure the interrupt IN endpoint ----------------
    let ep_num = (kbd.endpoint_addr & 0x0F) as u32;
    let dci = ep_num * 2 + 1; // IN endpoint
    let ep_index = (dci - 1) as usize;

    INPUT_CONTEXT.control.drop_flags = 0;
    INPUT_CONTEXT.control.add_flags = 1 | (1 << dci);
    INPUT_CONTEXT.slot.dword0 = (dci << 27) | (speed << 20); // Context Entries = highest DCI
    INPUT_CONTEXT.slot.dword1 = (port as u32) << 16;

    let ring = ep1_in_ring_base();
    let mps = kbd.max_packet as u32;
    INPUT_CONTEXT.ep[ep_index].dword0 = interrupt_interval(speed, kbd.interval) << 16;
    INPUT_CONTEXT.ep[ep_index].dword1 = (3u32 << 1) | (7u32 << 3) | (mps << 16); // Interrupt IN
    INPUT_CONTEXT.ep[ep_index].deq_ptr_lo = (ring as u32) | 1;
    INPUT_CONTEXT.ep[ep_index].deq_ptr_hi = (ring >> 32) as u32;
    INPUT_CONTEXT.ep[ep_index].dword4 = (mps << 16) | 8; // Max ESIT Payload, Avg TRB Length

    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_configure_endpoint_command(slot_id, input_context_base())?;

    let info = HidKeyboardInfo {
        slot_id,
        endpoint_addr: kbd.endpoint_addr,
        max_packet: kbd.max_packet,
        interval: kbd.interval,
    };
    crate::framebuffer::write_str("USB: keyboard ready on port ");
    print_decimal(port as u32);
    crate::framebuffer::write_str("\n");
    Ok(info)
}

/// Boot keyboard interface found in a configuration descriptor
struct BootKeyboard {
    interface: u8,
    endpoint_addr: u8,
    max_packet: u16,
    interval: u8,
}

/// Walk a configuration descriptor and return the first HID boot keyboard
/// interface together with its interrupt IN endpoint.
fn find_boot_keyboard(desc: &[u8]) -> Option<BootKeyboard> {
    let mut i = 0;
    let mut in_keyboard_iface: Option<u8> = None;
    while i + 2 <= desc.len() {
        let len = desc[i] as usize;
        if len < 2 || i + len > desc.len() {
            break;
        }
        match desc[i + 1] {
            USB_DT_INTERFACE if len >= 9 => {
                let is_kbd = desc[i + 5] == USB_CLASS_HID
                    && desc[i + 6] == USB_HID_SUBCLASS_BOOT
                    && desc[i + 7] == USB_HID_PROTOCOL_KEYBOARD;
                in_keyboard_iface = if is_kbd { Some(desc[i + 2]) } else { None };
            }
            USB_DT_ENDPOINT if len >= 7 => {
                if let Some(iface) = in_keyboard_iface {
                    let addr = desc[i + 2];
                    let attrs = desc[i + 3];
                    if addr & USB_DIR_IN != 0 && attrs & 0x03 == USB_ENDPOINT_XFER_INT {
                        return Some(BootKeyboard {
                            interface: iface,
                            endpoint_addr: addr,
                            max_packet: ((desc[i + 5] as u16) << 8 | desc[i + 4] as u16) & 0x7FF,
                            interval: desc[i + 6],
                        });
                    }
                }
            }
            _ => {}
        }
        i += len;
    }
    None
}

/// Busy-wait for roughly `n` iterations
fn spin(n: u32) {
    for _ in 0..n {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

/// Print a u32 in decimal
fn print_decimal(mut value: u32) {
    let mut digits = [0u8; 10];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (value % 10) as u8;
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for i in (0..len).rev() {
        crate::framebuffer::write_str(core::str::from_utf8(&[digits[i]]).unwrap());
    }
}

/// Read HID keyboard report via interrupt transfer (polling).
///
/// Keeps one Normal TRB outstanding on the keyboard's interrupt IN endpoint.
/// Returns the number of bytes received (0 if no new report yet).
pub unsafe fn read_keyboard_report() -> usize {
    use crate::usb::xhci;

    let keyboard = match get_hid_keyboard() {
        Some(k) => k,
        None => return 0,
    };
    let controller = match xhci::controller() {
        Some(c) => c,
        None => return 0,
    };

    if !TRANSFER_ACTIVE {
        // Normal TRB: transfer length 8, IOC + ISP
        let trb = NormalTrb {
            data_ptr: get_report_buffer_ptr(),
            status: (HID_REPORT_SIZE as u32) & 0x1_FFFF,
            control: (1 << 10) | (1 << 5) | (1 << 2),
        };
        if ep1_in_ring_enqueue(&trb).is_err() {
            return 0;
        }
        let dci = (keyboard.endpoint_addr & 0x0F) * 2 + 1;
        controller.ring_doorbell(keyboard.slot_id, dci, 0);
        TRANSFER_ACTIVE = true;
    }

    // Check (briefly) for the transfer event
    for _ in 0..TRANSFER_TIMEOUT {
        if let Some(event) = controller.poll_events() {
            let event_type = ((event.control >> 10) & 0x3F) as u8;
            let slot = ((event.control >> 24) & 0xFF) as u8;
            if event_type != 32 || slot != keyboard.slot_id {
                continue; // e.g. Port Status Change events
            }
            TRANSFER_ACTIVE = false;
            let code = (event.status >> 24) as u8;
            if code != 1 && code != 13 {
                return 0;
            }
            // Event status holds the residual (bytes NOT transferred)
            let residual = (event.status & 0x00FF_FFFF) as usize;
            return HID_REPORT_SIZE.saturating_sub(residual);
        }
        core::arch::asm!("nop", options(nomem, nostack));
    }
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
pub fn set_boot_protocol_setup_packet(interface: u8) -> UsbSetupPacket {
    UsbSetupPacket {
        bm_request_type: 0x21,  // Host-to-device, Type=Class, Recipient=Interface
        b_request: USB_REQ_SET_PROTOCOL,
        w_value: USB_HID_BOOT_PROTOCOL as u16,
        w_index: interface as u16,
        w_length: 0,
    }
}

/// Create a SET_IDLE (duration 0: report only on change) setup packet
pub fn set_idle_setup_packet(interface: u8) -> UsbSetupPacket {
    UsbSetupPacket {
        bm_request_type: 0x21,
        b_request: USB_REQ_SET_IDLE,
        w_value: 0,
        w_index: interface as u16,
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
