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
}

/// Event Ring - must be 64-byte aligned
#[repr(C, align(64))]
pub struct EventRingStruct {
    pub trbs: [EventTrb; TRB_RING_SIZE],
    pub dequeue: AtomicU32,
    pub cycle_state: AtomicU32,
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
}

/// DCBAA - must be 64-byte aligned
#[repr(C, align(64))]
pub struct DcbaaStruct {
    pub data: [u64; 256],
}

/// Static allocations
pub static mut COMMAND_RING_DATA: CommandRingStruct = CommandRingStruct {
    trbs: [NormalTrb { data_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
    enqueue: AtomicU32::new(0),
    dequeue: AtomicU32::new(0),
    cycle_state: AtomicU32::new(1),
};

pub static mut EVENT_RING_DATA: EventRingStruct = EventRingStruct {
    trbs: [EventTrb { trb_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
    dequeue: AtomicU32::new(0),
    cycle_state: AtomicU32::new(1),
};

pub static mut ERST_DATA: ErstStruct = ErstStruct {
    entries: [ErstEntry {
        segment_base: 0,
        segment_size: TRB_RING_SIZE as u32,
        _reserved: 0,
    }],
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
        }
    }

    #[test]
    fn test_ring_sizes() {
        assert_eq!(TRB_RING_SIZE, 16);
        assert_eq!(core::mem::size_of::<NormalTrb>(), 16);
        assert_eq!(core::mem::size_of::<EventTrb>(), 16);
    }
}
