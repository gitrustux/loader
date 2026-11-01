// Copyright 2025 The Rustux Authors
//
// DMA Allocator for xHCI
//
// xHCI requires all DMA structures to be 64-byte aligned.
// This module provides aligned DMA buffers using explicit alignment.

extern crate alloc;
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

/// Enqueue a TRB on a producer ring of `size` TRBs (command ring or
/// transfer ring) and return the physical address of the TRB written.
///
/// The last slot of every producer ring holds a Link TRB that points back to
/// the start of the ring with Toggle Cycle set, so the controller follows the
/// ring around instead of running off the end of the array. Software does not
/// track completions here, so there is no "ring full" check; the rings are
/// only ever a few TRBs ahead of the controller.
pub unsafe fn producer_ring_enqueue_sized(
    trbs: *mut NormalTrb,
    size: usize,
    enqueue: &AtomicU32,
    cycle_state: &AtomicU32,
    trb: &NormalTrb,
) -> u64 {
    let mut index = enqueue.load(Ordering::Acquire) as usize;
    let mut cycle = cycle_state.load(Ordering::Acquire) & 1;

    if index >= size - 1 {
        // Write the Link TRB in the last slot, hand it to the controller with
        // the current cycle bit, then wrap and flip our cycle state.
        write_trb(
            trbs.add(size - 1),
            trbs as u64,
            0,
            (6 << 10) | TRB_TOGGLE_CYCLE_BIT,
            cycle,
        );
        index = 0;
        cycle ^= 1;
        cycle_state.store(cycle, Ordering::Release);
    }

    write_trb(trbs.add(index), trb.data_ptr, trb.status, trb.control, cycle);

    enqueue.store((index + 1) as u32, Ordering::Release);
    trbs.add(index) as u64
}

/// [`producer_ring_enqueue_sized`] for the fixed-size static rings
pub unsafe fn producer_ring_enqueue(
    trbs: *mut NormalTrb,
    enqueue: &AtomicU32,
    cycle_state: &AtomicU32,
    trb: &NormalTrb,
) -> u64 {
    producer_ring_enqueue_sized(trbs, TRB_RING_SIZE, enqueue, cycle_state, trb)
}

/// Write a TRB so that the controller only sees it once it is complete:
/// first with the cycle bit the controller does NOT own, then flip the
/// control word to the producer cycle state.
unsafe fn write_trb(slot: *mut NormalTrb, data_ptr: u64, status: u32, control: u32, cycle: u32) {
    let control = control & !TRB_CYCLE_BIT;
    core::ptr::write_volatile(
        slot,
        NormalTrb { data_ptr, status, control: control | (cycle ^ 1) },
    );
    core::sync::atomic::fence(Ordering::SeqCst);
    // The control dword sits at offset 12 of a 16-byte TRB in a 64-byte aligned ring
    let control_ptr = core::ptr::addr_of_mut!((*slot).control);
    core::ptr::write_volatile(control_ptr, control | cycle);
    core::sync::atomic::fence(Ordering::SeqCst);
}

/// Enqueue to command ring. Returns the TRB's address (used to match its
/// Command Completion Event) and the cycle bit it was written with.
pub unsafe fn command_ring_enqueue(trb: &NormalTrb) -> Result<(u64, u32), UsbError> {
    let ring = &mut COMMAND_RING_DATA;
    let addr = producer_ring_enqueue(ring.trbs.as_mut_ptr(), &ring.enqueue, &ring.cycle_state, trb);
    let cycle = (core::ptr::read_volatile(addr as *const NormalTrb).control & TRB_CYCLE_BIT) as u32;
    Ok((addr, cycle))
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

    // Written by the controller: read it with a volatile access
    let trb = core::ptr::read_volatile(&ring.trbs[index]);
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

// ============================================================
// PER-DEVICE MEMORY AND TRANSFER RINGS
// ============================================================
//
// kernel-efi runs on the firmware's identity mapping, so the address of
// any kernel allocation is also its physical address. Device structures
// get whole 4 KiB pages: that satisfies every xHCI alignment rule (64-byte
// contexts and rings, no 64 KiB boundary crossing).

/// Allocate a zeroed, 4 KiB aligned page for device DMA. The kernel heap
/// never frees, so pages are kept and reused per slot.
pub fn alloc_dma_page() -> Option<*mut u8> {
    let layout = core::alloc::Layout::from_size_align(4096, 4096).ok()?;
    let p = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if p.is_null() { None } else { Some(p) }
}

/// A transfer ring occupying one page: 255 TRBs plus the Link TRB
pub struct Ring {
    trbs: *mut NormalTrb,
    enqueue: AtomicU32,
    cycle_state: AtomicU32,
}

/// TRBs per ring page
pub const RING_TRBS: usize = 4096 / 16;

impl Ring {
    /// A ring in a freshly zeroed page
    pub fn new() -> Option<Self> {
        let page = alloc_dma_page()?;
        Some(Self {
            trbs: page as *mut NormalTrb,
            enqueue: AtomicU32::new(0),
            cycle_state: AtomicU32::new(1),
        })
    }

    /// Empty the ring for reuse (contents zeroed, cycle state 1)
    pub fn reset(&mut self) {
        unsafe { core::ptr::write_bytes(self.trbs as *mut u8, 0, 4096) };
        self.enqueue.store(0, Ordering::Release);
        self.cycle_state.store(1, Ordering::Release);
    }

    /// Physical address of the first TRB (for endpoint contexts)
    pub fn base(&self) -> u64 {
        self.trbs as u64
    }

    /// Enqueue a TRB; returns its address
    pub fn push(&mut self, trb: &NormalTrb) -> u64 {
        unsafe { producer_ring_enqueue_sized(self.trbs, RING_TRBS, &self.enqueue, &self.cycle_state, trb) }
    }
}

/// ============================================================
/// SCRATCHPAD BUFFERS
/// ============================================================
/// Many controllers (all recent Intel chipsets) require the OS to provide
/// "scratchpad" pages (HCSPARAMS2). DCBAA[0] must point to an array of
/// page addresses before any slot is enabled.

/// Maximum scratchpad pages we can provide (4 KiB each)
pub const MAX_SCRATCHPAD_PAGES: usize = 64;

#[repr(C, align(4096))]
pub struct ScratchpadPages {
    pub pages: [[u8; 4096]; MAX_SCRATCHPAD_PAGES],
}

#[repr(C, align(64))]
pub struct ScratchpadArray {
    pub entries: [u64; MAX_SCRATCHPAD_PAGES],
}

pub static mut SCRATCHPAD_PAGES: ScratchpadPages = ScratchpadPages {
    pages: [[0; 4096]; MAX_SCRATCHPAD_PAGES],
};

pub static mut SCRATCHPAD_ARRAY: ScratchpadArray = ScratchpadArray {
    entries: [0; MAX_SCRATCHPAD_PAGES],
};

/// Fill the scratchpad array with `count` page addresses and point DCBAA[0]
/// at it. Returns false if more pages are required than we have.
pub unsafe fn setup_scratchpad(count: usize) -> bool {
    if count == 0 {
        DCBAA_DATA.data[0] = 0;
        return true;
    }
    if count > MAX_SCRATCHPAD_PAGES {
        return false;
    }
    for i in 0..count {
        SCRATCHPAD_ARRAY.entries[i] = SCRATCHPAD_PAGES.pages[i].as_ptr() as u64;
    }
    DCBAA_DATA.data[0] = SCRATCHPAD_ARRAY.entries.as_ptr() as u64;
    true
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

    #[test]
    fn test_context_sizes() {
        assert_eq!(core::mem::size_of::<SlotContext>(), 32);
        assert_eq!(core::mem::size_of::<EndpointContext>(), 32);
        assert_eq!(core::mem::size_of::<InputControlContext>(), 32);
    }
}
