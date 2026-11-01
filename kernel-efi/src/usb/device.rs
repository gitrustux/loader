// Copyright 2025 The Rustux Authors
//
// USB device enumeration: root ports, USB 2.0 hubs, HID boot keyboards
//
// Every connected root port is enumerated. Hubs are configured and the
// devices behind them enumerated in turn (route string, and for full/low
// speed devices behind a high-speed hub the TT fields). Every HID boot
// keyboard found is kept (up to MAX_KEYBOARDS); other devices are released.
//
// Each slot has its own Input Context, Device Context, EP0 ring and
// descriptor buffer (one page each), so a hub stays configured while the
// devices behind it are set up. kernel-efi runs identity-mapped, so the
// address of a page is also its physical address.

use crate::usb::{UsbError, trb::*};
use crate::usb::xhci::{
    self, PORTSC_CCS, PORTSC_PED, PORTSC_PORT_SPEED_MASK, PORTSC_PORT_SPEED_SHIFT,
    SPEED_FULL, SPEED_LOW, SPEED_HIGH, SPEED_SUPER,
};
use crate::usb::dma::{
    alloc_dma_page, DeviceContextStruct, InputContextStruct, Ring, DCBAA_DATA,
};
use crate::time::delay_ms;

/// USB HID Boot Protocol keyboard report size
pub const HID_REPORT_SIZE: usize = 8;

/// USB control request types
pub const USB_REQ_GET_STATUS: u8 = 0x00;
pub const USB_REQ_CLEAR_FEATURE: u8 = 0x01;
pub const USB_REQ_SET_FEATURE: u8 = 0x03;
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
pub const USB_DT_HUB: u8 = 0x29;

/// USB HID protocol
pub const USB_HID_BOOT_PROTOCOL: u8 = 0x00;
pub const USB_HID_REPORT_PROTOCOL: u8 = 0x01;

/// USB class codes
pub const USB_CLASS_HID: u8 = 0x03;
pub const USB_CLASS_HUB: u8 = 0x09;
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

/// Hub class features and port status bits (USB 2.0 ch. 11.24)
const HUB_PORT_RESET: u16 = 4;
const HUB_PORT_POWER: u16 = 8;
const HUB_C_PORT_CONNECTION: u16 = 16;
const HUB_C_PORT_RESET: u16 = 20;
const HUB_STAT_CONNECTION: u16 = 0x0001;
const HUB_STAT_ENABLE: u16 = 0x0002;
const HUB_STAT_LOW_SPEED: u16 = 0x0200;
const HUB_STAT_HIGH_SPEED: u16 = 0x0400;
const HUB_CHANGE_RESET: u16 = 0x0010;

/// Slots tracked (the controller is configured for at most 32)
const MAX_SLOTS: usize = 32;
/// Keyboards kept
const MAX_KEYBOARDS: usize = 4;
/// Hub nesting limit (USB allows 5 tiers of hubs)
const MAX_HUB_DEPTH: u8 = 4;

// ============================================================
// Per-slot memory
// ============================================================

struct SlotMem {
    input: *mut InputContextStruct,
    dev: *mut DeviceContextStruct,
    ep0: Ring,
    /// Descriptor / status buffer (one page)
    buf: *mut u8,
}

const NO_SLOT: Option<SlotMem> = None;
static mut SLOT_MEM: [Option<SlotMem>; MAX_SLOTS + 1] = [NO_SLOT; MAX_SLOTS + 1];

/// Memory for `slot`, zeroed. Pages are allocated the first time a slot
/// id is used and reused afterwards (the kernel heap never frees).
unsafe fn slot_mem(slot: u8) -> Result<&'static mut SlotMem, UsbError> {
    let s = slot as usize;
    if s == 0 || s > MAX_SLOTS {
        return Err(UsbError::XhciInitFailed);
    }
    let entry = &mut *core::ptr::addr_of_mut!(SLOT_MEM[s]);
    match entry {
        Some(m) => {
            core::ptr::write_bytes(m.input as *mut u8, 0, 4096);
            core::ptr::write_bytes(m.dev as *mut u8, 0, 4096);
            core::ptr::write_bytes(m.buf, 0, 4096);
            m.ep0.reset();
        }
        None => {
            let input = alloc_dma_page().ok_or(UsbError::XhciInitFailed)? as *mut InputContextStruct;
            let dev = alloc_dma_page().ok_or(UsbError::XhciInitFailed)? as *mut DeviceContextStruct;
            let buf = alloc_dma_page().ok_or(UsbError::XhciInitFailed)?;
            let ep0 = Ring::new().ok_or(UsbError::XhciInitFailed)?;
            *entry = Some(SlotMem { input, dev, ep0, buf });
        }
    }
    Ok(entry.as_mut().unwrap())
}

// ============================================================
// Keyboards
// ============================================================

struct Keyboard {
    slot: u8,
    dci: u8,
    ring: Ring,
    buf: *mut u8,
    active: bool,
    dead: bool,
    prev: [u8; 6],
}

const NO_KEYBOARD: Option<Keyboard> = None;
static mut KEYBOARDS: [Option<Keyboard>; MAX_KEYBOARDS] = [NO_KEYBOARD; MAX_KEYBOARDS];
static mut KEYBOARD_COUNT: usize = 0;

/// Check if at least one USB keyboard is ready
pub fn has_hid_keyboard() -> bool {
    unsafe { KEYBOARD_COUNT > 0 }
}

/// Number of USB keyboards ready
pub fn keyboard_count() -> usize {
    unsafe { KEYBOARD_COUNT }
}

/// Kept for callers of the old single-keyboard API
pub fn xhci_succeeded_with_device() -> bool {
    has_hid_keyboard()
}

/// Keep transfers armed on every keyboard and decode finished reports.
/// Decoded characters go to `hid`'s queue.
pub unsafe fn poll_keyboards() {
    if KEYBOARD_COUNT == 0 {
        return;
    }
    let c = match xhci::controller_mut() {
        Some(c) => c,
        None => return,
    };

    // One Normal TRB outstanding per keyboard
    for kb in (*core::ptr::addr_of_mut!(KEYBOARDS)).iter_mut().flatten() {
        if !kb.active && !kb.dead {
            kb.ring.push(&NormalTrb {
                data_ptr: kb.buf as u64,
                status: HID_REPORT_SIZE as u32,
                control: (1 << 10) | (1 << 5) | (1 << 2), // Normal, IOC, ISP
            });
            c.ring_doorbell(kb.slot, kb.dci, 0);
            kb.active = true;
        }
    }

    // Dispatch transfer events to their keyboard
    for _ in 0..32 {
        let event = match c.poll_events() {
            Some(e) => e,
            None => break,
        };
        if (event.control >> 10) & 0x3F != 32 {
            continue; // port status changes etc.
        }
        let slot = ((event.control >> 24) & 0xFF) as u8;
        let ep = ((event.control >> 16) & 0x1F) as u8;
        let code = (event.status >> 24) as u8;
        let residual = (event.status & 0x00FF_FFFF) as usize;
        for kb in (*core::ptr::addr_of_mut!(KEYBOARDS)).iter_mut().flatten() {
            if kb.slot != slot || kb.dci != ep {
                continue;
            }
            kb.active = false;
            if code == 1 || code == 13 {
                let len = HID_REPORT_SIZE.saturating_sub(residual);
                let report = core::slice::from_raw_parts(kb.buf as *const u8, HID_REPORT_SIZE);
                crate::usb::hid::process_report(&report[..len], &mut kb.prev);
            } else {
                // The endpoint halts on errors; recovering it needs Reset
                // Endpoint + Set TR Dequeue Pointer, not implemented yet.
                kb.dead = true;
                crate::framebuffer::write_str("[USB] keyboard on slot ");
                print_decimal(slot as u32);
                crate::framebuffer::write_str(" stopped, CC=");
                print_decimal(code as u32);
                crate::framebuffer::write_str("\n");
            }
        }
    }
}

// ============================================================
// Enumeration
// ============================================================

/// Where a device is attached
#[derive(Clone, Copy)]
struct Location {
    /// Root hub port (1-based) the device or its first hub is on
    root_port: u8,
    /// Route string (xHCI 8.9): hub port numbers below the root port
    route: u32,
    /// Number of hubs between the root port and the device
    depth: u8,
    /// Device speed (xHCI protocol speed ID)
    speed: u32,
    /// Transaction translator: high-speed hub slot and port, for full/low
    /// speed devices behind a high-speed hub (0 = none)
    tt_slot: u8,
    tt_port: u8,
}

/// What a device turned out to be
enum Kind {
    Keyboard,
    Hub,
    Other,
}

/// Enumerate every connected root port, including devices behind hubs.
/// Returns the number of keyboards found.
pub unsafe fn enumerate_hid_keyboard() -> Result<usize, UsbError> {
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

        // USB2 ports need a port reset to become enabled; USB3 ports enable
        // themselves after link training.
        let speed = (portsc & PORTSC_PORT_SPEED_MASK) >> PORTSC_PORT_SPEED_SHIFT;
        if speed < SPEED_SUPER || portsc & PORTSC_PED == 0 {
            if controller.reset_port(port).is_err() {
                crate::framebuffer::write_str("port reset failed\n");
                continue;
            }
        }
        // Speed is only valid after reset for USB2
        let portsc = controller.read_port_sc(port);
        let speed = (portsc & PORTSC_PORT_SPEED_MASK) >> PORTSC_PORT_SPEED_SHIFT;
        crate::framebuffer::write_str(speed_name(speed));
        crate::framebuffer::write_str("-speed device\n");

        let loc = Location { root_port: port as u8, route: 0, depth: 0, speed, tt_slot: 0, tt_port: 0 };
        let _ = attach_device(loc);
        if xhci::controller().map_or(true, |c| c.is_dead()) {
            break;
        }
    }

    if KEYBOARD_COUNT > 0 {
        Ok(KEYBOARD_COUNT)
    } else if any_device {
        Err(UsbError::HidKeyboardNotFound)
    } else {
        Err(UsbError::DeviceNotFound)
    }
}

fn speed_name(speed: u32) -> &'static str {
    match speed {
        SPEED_FULL => "Full",
        SPEED_LOW => "Low",
        SPEED_HIGH => "High",
        SPEED_SUPER => "Super",
        5 => "SuperPlus",
        _ => "Unknown",
    }
}

fn indent(depth: u8) {
    for _ in 0..=depth {
        crate::framebuffer::write_str("  ");
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

/// Slot Context dwords 0-2 for a device at `loc`
fn slot_dwords(loc: &Location, context_entries: u32) -> (u32, u32, u32) {
    (
        (context_entries << 27) | (loc.speed << 20) | (loc.route & 0xF_FFFF),
        (loc.root_port as u32) << 16,
        (loc.tt_slot as u32) | ((loc.tt_port as u32) << 8),
    )
}

unsafe fn ctrl_get(slot: u8, m: &mut SlotMem, setup: UsbSetupPacket, len: u16) -> Result<usize, UsbError> {
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_control_get(slot, &mut m.ep0, &setup, m.buf as u64, len).map(|n| n as usize)
}

unsafe fn ctrl_set(slot: u8, m: &mut SlotMem, setup: UsbSetupPacket) -> Result<(), UsbError> {
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_control_set(slot, &mut m.ep0, &setup)
}

/// Enable a slot for the device at `loc`, address it, read its
/// descriptors and set it up as a keyboard or hub. Anything else is
/// released again.
unsafe fn attach_device(loc: Location) -> Result<Kind, UsbError> {
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    let slot = c.issue_enable_slot_command()?;
    match setup_device(slot, &loc) {
        Ok(Kind::Other) => {
            release_slot(slot);
            Ok(Kind::Other)
        }
        Ok(kind) => Ok(kind),
        Err(e) => {
            release_slot(slot);
            Err(e)
        }
    }
}

unsafe fn release_slot(slot: u8) {
    if let Some(c) = xhci::controller_mut() {
        let _ = c.issue_disable_slot_command(slot);
    }
    DCBAA_DATA.data[slot as usize] = 0;
}

unsafe fn setup_device(slot: u8, loc: &Location) -> Result<Kind, UsbError> {
    let m = slot_mem(slot)?;
    let input = &mut *m.input;

    // ---------------- Address Device ----------------
    DCBAA_DATA.data[slot as usize] = m.dev as u64;
    let mut ep0_mps = default_ep0_mps(loc.speed);
    let (d0, d1, d2) = slot_dwords(loc, 1);
    input.control.drop_flags = 0;
    input.control.add_flags = 0x3; // Slot + EP0
    input.slot.dword0 = d0;
    input.slot.dword1 = d1;
    input.slot.dword2 = d2;
    input.slot.dword3 = 0;
    let ep0_ring = m.ep0.base();
    input.ep[0].dword0 = 0;
    input.ep[0].dword1 = (3u32 << 1) | (4u32 << 3) | (ep0_mps << 16); // CErr=3, Control
    input.ep[0].deq_ptr_lo = (ep0_ring as u32) | 1; // DCS=1
    input.ep[0].deq_ptr_hi = (ep0_ring >> 32) as u32;
    input.ep[0].dword4 = 8; // Average TRB Length

    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_address_device_command(slot, m.input as u64, false)?;
    delay_ms(2); // SET_ADDRESS recovery

    // ---------------- Device descriptor ----------------
    // The first 8 bytes give bMaxPacketSize0
    ctrl_get(slot, m, get_descriptor_setup_packet(USB_DT_DEVICE, 0, 8), 8)?;
    let mps0 = *m.buf.add(7) as u32;
    if loc.speed < SPEED_SUPER && mps0 != 0 && mps0 != ep0_mps {
        ep0_mps = mps0;
        let input = &mut *m.input;
        input.control.add_flags = 0x2; // EP0 only
        input.ep[0].dword1 = (3u32 << 1) | (4u32 << 3) | (ep0_mps << 16);
        let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
        c.issue_evaluate_context_command(slot, m.input as u64)?;
    }
    ctrl_get(slot, m, get_descriptor_setup_packet(USB_DT_DEVICE, 0, 18), 18)?;
    let dd = core::slice::from_raw_parts(m.buf as *const u8, 18);
    let vid = (dd[9] as u16) << 8 | dd[8] as u16;
    let pid = (dd[11] as u16) << 8 | dd[10] as u16;
    let dev_class = dd[4];
    indent(loc.depth);
    crate::framebuffer::write_str("device ");
    print_hex_word(vid);
    crate::framebuffer::write_str(":");
    print_hex_word(pid);
    crate::framebuffer::write_str(" class=");
    print_hex_byte(dev_class);
    crate::framebuffer::write_str("\n");

    // ---------------- Configuration descriptor ----------------
    ctrl_get(slot, m, get_descriptor_setup_packet(USB_DT_CONFIG, 0, 9), 9)?;
    let total = core::cmp::min((*m.buf.add(3) as usize) << 8 | *m.buf.add(2) as usize, 4096);
    let got = ctrl_get(slot, m, get_descriptor_setup_packet(USB_DT_CONFIG, 0, total as u16), total as u16)?;
    let mut cfg = [0u8; 512];
    let cfg_len = core::cmp::min(core::cmp::min(got, total), cfg.len());
    cfg[..cfg_len].copy_from_slice(core::slice::from_raw_parts(m.buf as *const u8, cfg_len));
    let cfg = &cfg[..cfg_len];
    let config_value = if cfg.len() > 5 { cfg[5] } else { 1 };

    // ---------------- Hub ----------------
    if dev_class == USB_CLASS_HUB || has_interface_class(cfg, USB_CLASS_HUB) {
        if loc.speed >= SPEED_SUPER {
            // A USB 3 hub also appears as a USB 2 hub on a USB 2 port;
            // keyboards are found there.
            indent(loc.depth);
            crate::framebuffer::write_str("USB 3 hub: using its USB 2 side instead\n");
            return Ok(Kind::Other);
        }
        if loc.depth >= MAX_HUB_DEPTH {
            indent(loc.depth);
            crate::framebuffer::write_str("hub nested too deep, skipping\n");
            return Ok(Kind::Other);
        }
        setup_hub(slot, m, loc, config_value, dd[6])?;
        return Ok(Kind::Hub);
    }

    // ---------------- Keyboard ----------------
    let kbd = match find_boot_keyboard(cfg) {
        Some(k) => k,
        None => {
            indent(loc.depth);
            crate::framebuffer::write_str("not a boot keyboard, skipping\n");
            return Ok(Kind::Other);
        }
    };
    if KEYBOARD_COUNT >= MAX_KEYBOARDS {
        indent(loc.depth);
        crate::framebuffer::write_str("boot keyboard (limit reached, skipping)\n");
        return Ok(Kind::Other);
    }
    indent(loc.depth);
    crate::framebuffer::write_str("boot keyboard: interface ");
    print_decimal(kbd.interface as u32);
    crate::framebuffer::write_str(", endpoint 0x");
    print_hex_byte(kbd.endpoint_addr);
    crate::framebuffer::write_str("\n");

    ctrl_set(slot, m, set_configuration_setup_packet(config_value))?;
    if ctrl_set(slot, m, set_boot_protocol_setup_packet(kbd.interface)).is_err() {
        indent(loc.depth);
        crate::framebuffer::write_str("SET_PROTOCOL failed (continuing)\n");
    }
    let _ = ctrl_set(slot, m, set_idle_setup_packet(kbd.interface)); // optional

    // Configure the interrupt IN endpoint
    let ring = Ring::new().ok_or(UsbError::XhciInitFailed)?;
    let report_buf = alloc_dma_page().ok_or(UsbError::XhciInitFailed)?;
    let ep_num = (kbd.endpoint_addr & 0x0F) as u32;
    let dci = ep_num * 2 + 1; // IN endpoint
    let ep_index = (dci - 1) as usize;
    let input = &mut *m.input;
    let (d0, d1, d2) = slot_dwords(loc, dci); // Context Entries = highest DCI
    input.control.drop_flags = 0;
    input.control.add_flags = 1 | (1 << dci);
    input.slot.dword0 = d0;
    input.slot.dword1 = d1;
    input.slot.dword2 = d2;
    let mps = kbd.max_packet as u32;
    let ring_base = ring.base();
    input.ep[ep_index].dword0 = interrupt_interval(loc.speed, kbd.interval) << 16;
    input.ep[ep_index].dword1 = (3u32 << 1) | (7u32 << 3) | (mps << 16); // Interrupt IN
    input.ep[ep_index].deq_ptr_lo = (ring_base as u32) | 1;
    input.ep[ep_index].deq_ptr_hi = (ring_base >> 32) as u32;
    input.ep[ep_index].dword4 = (mps << 16) | 8; // Max ESIT Payload, Avg TRB Length
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    c.issue_configure_endpoint_command(slot, m.input as u64)?;

    KEYBOARDS[KEYBOARD_COUNT] = Some(Keyboard {
        slot,
        dci: dci as u8,
        ring,
        buf: report_buf,
        active: false,
        dead: false,
        prev: [0; 6],
    });
    KEYBOARD_COUNT += 1;
    indent(loc.depth);
    crate::framebuffer::write_str("keyboard ready\n");
    Ok(Kind::Keyboard)
}

/// Configure a USB 2.0 hub in `slot` and enumerate the devices on its ports.
unsafe fn setup_hub(slot: u8, m: &mut SlotMem, loc: &Location, config_value: u8, protocol: u8) -> Result<(), UsbError> {
    ctrl_set(slot, m, set_configuration_setup_packet(config_value))?;

    // Hub descriptor: bNbrPorts, wHubCharacteristics, bPwrOn2PwrGood
    let n = ctrl_get(
        slot,
        m,
        UsbSetupPacket { bm_request_type: 0xA0, b_request: USB_REQ_GET_DESCRIPTOR, w_value: (USB_DT_HUB as u16) << 8, w_index: 0, w_length: 16 },
        16,
    )?;
    if n < 7 {
        return Err(UsbError::XhciInitFailed);
    }
    let ports = *m.buf.add(2) as u32;
    let characteristics = (*m.buf.add(4) as u32) << 8 | *m.buf.add(3) as u32;
    let power_good_ms = (*m.buf.add(5) as u64) * 2;
    let think_time = (characteristics >> 5) & 0x3;
    indent(loc.depth);
    crate::framebuffer::write_str("hub with ");
    print_decimal(ports);
    crate::framebuffer::write_str(" ports");
    if protocol == 2 {
        crate::framebuffer::write_str(" (multi-TT, using single TT)");
    }
    crate::framebuffer::write_str("\n");

    // Tell the controller this slot is a hub (Hub, Number of Ports, TT
    // Think Time); Configure Endpoint with only the Slot Context added.
    let input = &mut *m.input;
    let (d0, d1, d2) = slot_dwords(loc, 1);
    input.control.drop_flags = 0;
    input.control.add_flags = 1;
    input.slot.dword0 = d0 | (1 << 26);
    input.slot.dword1 = d1 | (ports << 24);
    input.slot.dword2 = d2 | if loc.speed == SPEED_HIGH { think_time << 16 } else { 0 };
    let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
    if c.issue_configure_endpoint_command(slot, m.input as u64).is_err() {
        let c = xhci::controller_mut().ok_or(UsbError::XhciNotFound)?;
        c.issue_evaluate_context_command(slot, m.input as u64)?;
    }

    // Power every port, then wait for power to be good plus debounce
    for p in 1..=ports {
        let _ = ctrl_set(slot, m, hub_feature(USB_REQ_SET_FEATURE, HUB_PORT_POWER, p));
    }
    delay_ms(core::cmp::max(power_good_ms, 20) + 100);

    for p in 1..=ports {
        let (status, _) = match hub_port_status(slot, m, p) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if status & HUB_STAT_CONNECTION == 0 {
            continue;
        }
        let _ = ctrl_set(slot, m, hub_feature(USB_REQ_CLEAR_FEATURE, HUB_C_PORT_CONNECTION, p));

        // Reset the port and wait for C_PORT_RESET
        indent(loc.depth);
        crate::framebuffer::write_str("hub port ");
        print_decimal(p);
        crate::framebuffer::write_str(": ");
        if ctrl_set(slot, m, hub_feature(USB_REQ_SET_FEATURE, HUB_PORT_RESET, p)).is_err() {
            crate::framebuffer::write_str("reset request failed\n");
            continue;
        }
        let mut reset_done = false;
        for _ in 0..50 {
            delay_ms(10);
            if let Ok((_, change)) = hub_port_status(slot, m, p) {
                if change & HUB_CHANGE_RESET != 0 {
                    reset_done = true;
                    break;
                }
            }
        }
        let _ = ctrl_set(slot, m, hub_feature(USB_REQ_CLEAR_FEATURE, HUB_C_PORT_RESET, p));
        if !reset_done {
            crate::framebuffer::write_str("reset timed out\n");
            continue;
        }
        delay_ms(10); // reset recovery (TRSTRCY)

        let status = match hub_port_status(slot, m, p) {
            Ok((s, _)) => s,
            Err(_) => continue,
        };
        if status & HUB_STAT_ENABLE == 0 {
            crate::framebuffer::write_str("not enabled after reset\n");
            continue;
        }
        let speed = if status & HUB_STAT_LOW_SPEED != 0 {
            SPEED_LOW
        } else if status & HUB_STAT_HIGH_SPEED != 0 {
            SPEED_HIGH
        } else {
            SPEED_FULL
        };
        crate::framebuffer::write_str(speed_name(speed));
        crate::framebuffer::write_str("-speed device\n");

        // Full/low speed devices behind a high-speed hub go through its
        // transaction translator; deeper down they inherit the same TT.
        let (tt_slot, tt_port) = if speed == SPEED_HIGH {
            (0, 0)
        } else if loc.speed == SPEED_HIGH {
            (slot, p as u8)
        } else {
            (loc.tt_slot, loc.tt_port)
        };
        let child = Location {
            root_port: loc.root_port,
            route: loc.route | (core::cmp::min(p, 15) << (4 * loc.depth as u32)),
            depth: loc.depth + 1,
            speed,
            tt_slot,
            tt_port,
        };
        let _ = attach_device(child);
        if xhci::controller().map_or(true, |c| c.is_dead()) {
            break;
        }
    }
    Ok(())
}

fn hub_feature(request: u8, feature: u16, port: u32) -> UsbSetupPacket {
    UsbSetupPacket { bm_request_type: 0x23, b_request: request, w_value: feature, w_index: port as u16, w_length: 0 }
}

/// GET_STATUS for a hub port: (wPortStatus, wPortChange)
unsafe fn hub_port_status(slot: u8, m: &mut SlotMem, port: u32) -> Result<(u16, u16), UsbError> {
    let setup = UsbSetupPacket { bm_request_type: 0xA3, b_request: USB_REQ_GET_STATUS, w_value: 0, w_index: port as u16, w_length: 4 };
    if ctrl_get(slot, m, setup, 4)? < 4 {
        return Err(UsbError::XhciInitFailed);
    }
    let b = core::slice::from_raw_parts(m.buf as *const u8, 4);
    Ok(((b[1] as u16) << 8 | b[0] as u16, (b[3] as u16) << 8 | b[2] as u16))
}

/// Does the configuration have an interface of `class`?
fn has_interface_class(desc: &[u8], class: u8) -> bool {
    let mut i = 0;
    while i + 2 <= desc.len() {
        let len = desc[i] as usize;
        if len < 2 || i + len > desc.len() {
            break;
        }
        if desc[i + 1] == USB_DT_INTERFACE && len >= 9 && desc[i + 5] == class {
            return true;
        }
        i += len;
    }
    false
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

fn print_hex_byte(value: u8) {
    let h = b"0123456789ABCDEF";
    let b = [h[(value >> 4) as usize], h[(value & 0xF) as usize]];
    crate::framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&b) });
}

fn print_hex_word(value: u16) {
    print_hex_byte((value >> 8) as u8);
    print_hex_byte(value as u8);
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
    digits[..len].reverse();
    crate::framebuffer::write_str(unsafe { core::str::from_utf8_unchecked(&digits[..len]) });
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

}
