// Copyright 2025 The Rustux Authors
//
// DMA Allocator for xHCI
//
// xHCI requires all DMA structures to be 64-byte aligned.
// This module provides aligned DMA buffers using explicit alignment.

use core::sync::atomic::{AtomicU32, Ordering};
use crate::usb::trb::*;  // Imports TRB_RING_SIZE, NormalTrb, EventTrb, etc.
use crate::usb::UsbError;  // Import UsbError type

/// ============================================================
/// GLOBAL ALLOCATIONS (placed at top for reliability)
/// ============================================================
/// These must be 64-byte aligned. We use large alignment
/// attributes and hope the linker respects them.

/// Command Ring - must be 64-byte aligned
#[repr(C, align(64))]
pub struct CommandRingStruct {
    pub trbs: [NormalTrb; TRB_RING_SIZE],
    pub enqueue: AtomicU32,
    pub dequeue: AtomicU32,
    pub cycle_state: AtomicU32,
    // Padding to prevent overlap with next allocation
    // Total size: 256 + 12 = 268 bytes, round up to 320 bytes (5 * 64)
    _padding: [u8; 52],
}

/// Event Ring - must be 64-byte aligned
#[repr(C, align(64))]
pub struct EventRingStruct {
    pub trbs: [EventTrb; TRB_RING_SIZE],
    pub dequeue: AtomicU32,
    pub cycle_state: AtomicU32,
    // Padding to prevent overlap with next allocation
    // Total size: 256 + 8 = 264 bytes, round up to 320 bytes (5 * 64)
    _padding: [u8; 56],
}

/// ERST Entry
#[repr(C)]
pub struct ErstEntry {
    pub segment_base: u64,
    pub segment_size: u32,
    _reserved: u32,
}

/// ERST - must be 64-byte aligned
#[repr(C, align(64))]
pub struct ErstStruct {
    pub entries: [ErstEntry; 1],
    // Padding to prevent overlap with next allocation
    // Total size: 16 bytes, round up to 64 bytes
    _padding: [u8; 48],
}

/// DCBAA - must be 64-byte aligned
#[repr(C, align(64))]
pub struct DcbaaStruct {
    pub data: [u64; 256],
    // No padding needed - this is the last (largest) allocation
}

/// Static allocations
pub static mut COMMAND_RING_DATA: CommandRingStruct = CommandRingStruct {
    trbs: [NormalTrb { data_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
    enqueue: AtomicU32::new(0),
    dequeue: AtomicU32::new(0),
    cycle_state: AtomicU32::new(1),
    _padding: [0; 52],
};

pub static mut EVENT_RING_DATA: EventRingStruct = EventRingStruct {
    trbs: [EventTrb { trb_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
    dequeue: AtomicU32::new(0),
    cycle_state: AtomicU32::new(1),
    _padding: [0; 56],
};

pub static mut ERST_DATA: ErstStruct = ErstStruct {
    entries: [ErstEntry {
        segment_base: 0,
        segment_size: TRB_RING_SIZE as u32,
        _reserved: 0,
    }],
    _padding: [0; 48],
};

pub static mut DCBAA_DATA: DcbaaStruct = DcbaaStruct {
    data: [0; 256],
};

/// ============================================================
/// ACCESSOR FUNCTIONS
/// ============================================================

/// Get command ring TRB base address (for CRCR)
pub fn command_ring_base() -> u64 {
    unsafe { &COMMAND_RING_DATA.trbs[0] as *const NormalTrb as u64 }
}

/// Get command ring cycle state
pub fn command_ring_cycle() -> u64 {
    unsafe { COMMAND_RING_DATA.cycle_state.load(Ordering::Acquire) as u64 }
}

/// Enqueue to command ring
pub unsafe fn command_ring_enqueue(trb: &NormalTrb) -> Result<(), UsbError> {
    let ring = &COMMAND_RING_DATA;
    let index = ring.enqueue.load(Ordering::Acquire) as usize;
    let cycle = ring.cycle_state.load(Ordering::Acquire);

    // Check if ring is full
    let next_index = (index + 1) % TRB_RING_SIZE;
    if next_index == ring.dequeue.load(Ordering::Acquire) as usize {
        return Err(UsbError::Timeout);
    }

    // Write TRB
    let trb_ptr = &ring.trbs[index] as *const NormalTrb as *mut NormalTrb;
    (*trb_ptr).data_ptr = trb.data_ptr;
    (*trb_ptr).status = trb.status;
    (*trb_ptr).control = trb.control | (cycle & 1);

    // Update enqueue pointer
    ring.enqueue.store(next_index as u32, Ordering::Release);

    // Toggle cycle bit if wrapping
    if next_index == 0 {
        let new_cycle = cycle ^ 1;
        ring.cycle_state.store(new_cycle, Ordering::Release);
    }

    Ok(())
}

/// Get command ring enqueue index
pub unsafe fn command_ring_enqueue_idx() -> u32 {
    COMMAND_RING_DATA.enqueue.load(Ordering::Acquire)
}

/// Get command ring cycle value
pub unsafe fn command_ring_cycle_value() -> u32 {
    COMMAND_RING_DATA.cycle_state.load(Ordering::Acquire)
}

/// Get event ring TRB base address
pub fn event_ring_base() -> u64 {
    unsafe { &EVENT_RING_DATA.trbs[0] as *const EventTrb as u64 }
}

/// Get event ring dequeue pointer
pub fn event_ring_dequeue_ptr() -> u64 {
    unsafe {
        let base = event_ring_base();
        let offset = EVENT_RING_DATA.dequeue.load(Ordering::Acquire) as u64 * 16;
        base + offset
    }
}

/// Dequeue from event ring
pub unsafe fn event_ring_dequeue() -> Option<EventTrb> {
    let ring = &EVENT_RING_DATA;
    let index = ring.dequeue.load(Ordering::Acquire) as usize;
    let cycle = ring.cycle_state.load(Ordering::Acquire);

    let trb = ring.trbs[index];
    let trb_cycle = (trb.control & TRB_CYCLE_BIT) as u32;

    // Check if TRB is owned by controller
    if trb_cycle != (cycle & 1) {
        return None;
    }

    // Update dequeue pointer
    let next_index = (index + 1) % TRB_RING_SIZE;
    ring.dequeue.store(next_index as u32, Ordering::Release);

    // Toggle cycle bit if wrapping
    if next_index == 0 {
        let new_cycle = cycle ^ 1;
        ring.cycle_state.store(new_cycle, Ordering::Release);
    }

    Some(trb)
}

/// Get event ring state
pub unsafe fn event_ring_state() -> (u32, u32) {
    let dequeue = EVENT_RING_DATA.dequeue.load(Ordering::Acquire);
    let cycle = EVENT_RING_DATA.cycle_state.load(Ordering::Acquire);
    (dequeue, cycle)
}

/// Get first event TRB (for debugging)
pub unsafe fn event_ring_first_trb() -> EventTrb {
    EVENT_RING_DATA.trbs[0]
}

/// Get ERST base address
pub fn erst_base() -> u64 {
    unsafe { &ERST_DATA.entries[0] as *const ErstEntry as u64 }
}

/// Initialize ERST with event ring address
pub unsafe fn erst_init() {
    ERST_DATA.entries[0].segment_base = event_ring_base();
    ERST_DATA.entries[0].segment_size = TRB_RING_SIZE as u32;
}

/// Get DCBAA base address
pub fn dcbaa_base() -> u64 {
    unsafe { &DCBAA_DATA.data[0] as *const u64 as u64 }
}

/// Alignment verification functions
pub fn command_ring_aligned() -> bool {
    command_ring_base() & 0x3F == 0
}

pub fn event_ring_aligned() -> bool {
    event_ring_base() & 0x3F == 0
}

pub fn erst_aligned() -> bool {
    erst_base() & 0x3F == 0
}

pub fn dcbaa_aligned() -> bool {
    dcbaa_base() & 0x3F == 0
}

// ============================================================
// DEVICE/INPUT CONTEXT AND ENDPOINT TRANSFER RINGS
// ============================================================

/// xHCI Slot Context (32 bytes, per xHCI spec Table 6-2)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct SlotContext {
    /// Route String[19:0], Speed[23:20], Context Entries[31:27]
    pub dword0: u32,
    /// Max Exit Latency[15:0], Root Hub Port Number[23:16]
    pub dword1: u32,
    /// TT Hub Slot ID[7:0], TT Port Number[15:8], Interrupter Target[31:22]
    pub dword2: u32,
    /// USB Device Address[7:0], Slot State[31:27]
    pub dword3: u32,
    pub _reserved: [u32; 4],
}

/// xHCI Endpoint Context (32 bytes, per xHCI spec Table 6-8)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct EndpointContext {
    /// EP State[2:0], Interval[23:16]
    pub dword0: u32,
    /// CErr[2:1], EP Type[5:3], Max Burst Size[15:8], Max Packet Size[31:16]
    pub dword1: u32,
    /// TR Dequeue Pointer Lo[31:4], DCS[0]
    pub deq_ptr_lo: u32,
    /// TR Dequeue Pointer Hi[31:0]
    pub deq_ptr_hi: u32,
    /// Average TRB Length[15:0]
    pub dword4: u32,
    pub _reserved: [u32; 3],
}

/// xHCI Input Control Context (32 bytes, per xHCI spec Table 6-10)
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct InputControlContext {
    /// Drop Context Flags: bit N = drop endpoint context DCI N
    pub drop_flags: u32,
    /// Add Context Flags: bit N = add/modify endpoint context DCI N
    pub add_flags: u32,
    pub _reserved: [u32; 6],
}

/// xHCI Input Context (1056 bytes, must be 64-byte aligned)
/// Layout: Input Control Context (32) + Slot Context (32) + 31 EP Contexts (992)
#[repr(C, align(64))]
pub struct InputContextStruct {
    pub control: InputControlContext,
    pub slot: SlotContext,
    /// ep[0] = DCI 1 (EP0 Control), ep[1] = DCI 2 (EP1 OUT), ep[2] = DCI 3 (EP1 IN), ...
    pub ep: [EndpointContext; 31],
}

/// xHCI Device Context (1024 bytes, must be 64-byte aligned)
/// Layout: Slot Context (32) + 31 EP Contexts (992)
/// This is written by the xHCI controller after Address Device command.
#[repr(C, align(64))]
pub struct DeviceContextStruct {
    pub slot: SlotContext,
    /// ep[0] = DCI 1 (EP0), ep[1] = DCI 2 (EP1 OUT), ep[2] = DCI 3 (EP1 IN), ...
    pub ep: [EndpointContext; 31],
}

/// EP0 Transfer Ring — Default Control Pipe for USB control transfers
#[repr(C, align(64))]
pub struct Ep0RingStruct {
    pub trbs: [NormalTrb; TRB_RING_SIZE],
    pub enqueue: AtomicU32,
    pub dequeue: AtomicU32,
    pub cycle_state: AtomicU32,
    pub _pad: [u8; 52],
}

/// EP1 IN Transfer Ring — HID keyboard interrupt IN endpoint
#[repr(C, align(64))]
pub struct Ep1InRingStruct {
    pub trbs: [NormalTrb; TRB_RING_SIZE],
    pub enqueue: AtomicU32,
    pub dequeue: AtomicU32,
    pub cycle_state: AtomicU32,
    pub _pad: [u8; 52],
}

const ZERO_SLOT: SlotContext = SlotContext {
    dword0: 0, dword1: 0, dword2: 0, dword3: 0, _reserved: [0; 4],
};
const ZERO_EP: EndpointContext = EndpointContext {
    dword0: 0, dword1: 0, deq_ptr_lo: 0, deq_ptr_hi: 0, dword4: 0, _reserved: [0; 3],
};

pub static mut INPUT_CONTEXT: InputContextStruct = InputContextStruct {
    control: InputControlContext { drop_flags: 0, add_flags: 0, _reserved: [0; 6] },
    slot: ZERO_SLOT,
    ep: [ZERO_EP; 31],
};

pub static mut DEVICE_CONTEXT: DeviceContextStruct = DeviceContextStruct {
    slot: ZERO_SLOT,
    ep: [ZERO_EP; 31],
};

pub static mut EP0_RING: Ep0RingStruct = Ep0RingStruct {
    trbs: [NormalTrb { data_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
    enqueue: AtomicU32::new(0),
    dequeue: AtomicU32::new(0),
    cycle_state: AtomicU32::new(1),
    _pad: [0; 52],
};

pub static mut EP1_IN_RING: Ep1InRingStruct = Ep1InRingStruct {
    trbs: [NormalTrb { data_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
    enqueue: AtomicU32::new(0),
    dequeue: AtomicU32::new(0),
    cycle_state: AtomicU32::new(1),
    _pad: [0; 52],
};

/// Get Input Context physical address
pub fn input_context_base() -> u64 {
    unsafe { &INPUT_CONTEXT as *const InputContextStruct as u64 }
}

/// Get Device Context physical address
pub fn device_context_base() -> u64 {
    unsafe { &DEVICE_CONTEXT as *const DeviceContextStruct as u64 }
}

/// Get EP0 Transfer Ring base address
pub fn ep0_ring_base() -> u64 {
    unsafe { &EP0_RING.trbs[0] as *const NormalTrb as u64 }
}

/// Get EP0 ring initial cycle state
pub fn ep0_ring_cycle() -> u64 {
    unsafe { EP0_RING.cycle_state.load(Ordering::Acquire) as u64 }
}

/// Enqueue a TRB to the EP0 transfer ring
pub unsafe fn ep0_ring_enqueue(trb: &NormalTrb) -> Result<(), UsbError> {
    let ring = &EP0_RING;
    let index = ring.enqueue.load(Ordering::Acquire) as usize;
    let cycle = ring.cycle_state.load(Ordering::Acquire);

    let next_index = (index + 1) % TRB_RING_SIZE;
    if next_index == ring.dequeue.load(Ordering::Acquire) as usize {
        return Err(UsbError::Timeout);
    }

    let trb_ptr = &ring.trbs[index] as *const NormalTrb as *mut NormalTrb;
    (*trb_ptr).data_ptr = trb.data_ptr;
    (*trb_ptr).status = trb.status;
    (*trb_ptr).control = trb.control | (cycle & 1);

    ring.enqueue.store(next_index as u32, Ordering::Release);

    if next_index == 0 {
        ring.cycle_state.store(cycle ^ 1, Ordering::Release);
    }
    Ok(())
}

/// Get EP1 IN Transfer Ring base address
pub fn ep1_in_ring_base() -> u64 {
    unsafe { &EP1_IN_RING.trbs[0] as *const NormalTrb as u64 }
}

/// Get EP1 IN ring initial cycle state
pub fn ep1_in_ring_cycle() -> u64 {
    unsafe { EP1_IN_RING.cycle_state.load(Ordering::Acquire) as u64 }
}

/// Enqueue a TRB to the EP1 IN transfer ring
pub unsafe fn ep1_in_ring_enqueue(trb: &NormalTrb) -> Result<(), UsbError> {
    let ring = &EP1_IN_RING;
    let index = ring.enqueue.load(Ordering::Acquire) as usize;
    let cycle = ring.cycle_state.load(Ordering::Acquire);

    let next_index = (index + 1) % TRB_RING_SIZE;
    if next_index == ring.dequeue.load(Ordering::Acquire) as usize {
        return Err(UsbError::Timeout);
    }

    let trb_ptr = &ring.trbs[index] as *const NormalTrb as *mut NormalTrb;
    (*trb_ptr).data_ptr = trb.data_ptr;
    (*trb_ptr).status = trb.status;
    (*trb_ptr).control = trb.control | (cycle & 1);

    ring.enqueue.store(next_index as u32, Ordering::Release);

    if next_index == 0 {
        ring.cycle_state.store(cycle ^ 1, Ordering::Release);
    }
    Ok(())
}

/// Alignment check helpers
pub fn input_context_aligned() -> bool {
    input_context_base() & 0x3F == 0
}
pub fn device_context_aligned() -> bool {
    device_context_base() & 0x3F == 0
}
pub fn ep0_ring_aligned() -> bool {
    ep0_ring_base() & 0x3F == 0
}
pub fn ep1_in_ring_aligned() -> bool {
    ep1_in_ring_base() & 0x3F == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alignments() {
        unsafe {
            assert!(command_ring_aligned());
            assert!(event_ring_aligned());
            assert!(erst_aligned());
            assert!(dcbaa_aligned());
            assert!(input_context_aligned());
            assert!(device_context_aligned());
            assert!(ep0_ring_aligned());
            assert!(ep1_in_ring_aligned());
        }
    }

    #[test]
    fn test_ring_sizes() {
        assert_eq!(TRB_RING_SIZE, 16);
        assert_eq!(core::mem::size_of::<NormalTrb>(), 16);
        assert_eq!(core::mem::size_of::<EventTrb>(), 16);
    }

    #[test]
    fn test_context_sizes() {
        assert_eq!(core::mem::size_of::<SlotContext>(), 32);
        assert_eq!(core::mem::size_of::<EndpointContext>(), 32);
        assert_eq!(core::mem::size_of::<InputControlContext>(), 32);
    }
}
