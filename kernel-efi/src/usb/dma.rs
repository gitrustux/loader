// Copyright 2025 The Rustux Authors
//
// DMA Allocator for xHCI
//
// xHCI requires all DMA structures to be 64-byte aligned.
// Standard #[repr(align(64))] does NOT guarantee static allocation alignment.
// This module provides explicitly aligned DMA buffers.

use core::sync::atomic::{AtomicU32, Ordering};
use crate::usb::trb::*;  // Imports TRB_RING_SIZE, NormalTrb, EventTrb, etc.
use crate::usb::UsbError;  // Import UsbError type

/// Round up to 64-byte alignment
const fn align_up_64(addr: usize) -> usize {
    (addr + 63) & !63
}

/// ============================================================
/// ALIGNED DMA BUFFERS
/// ============================================================
/// All buffers are manually aligned to 64 bytes using padding.
/// The linker will place the entire struct, and we use the
/// aligned field directly.

/// Command Ring Buffer (must be 64-byte aligned)
#[repr(C, align(64))]
pub struct AlignedCommandRing {
    /// Padding to force alignment of the data field
    _padding: [u8; 64],
    /// TRB data (16 bytes each)
    pub data: [NormalTrb; TRB_RING_SIZE],
    /// Enqueue pointer
    pub enqueue: AtomicU32,
    /// Dequeue pointer
    pub dequeue: AtomicU32,
    /// Cycle state
    pub cycle_state: AtomicU32,
}

impl AlignedCommandRing {
    /// Create a new aligned command ring
    pub const fn new() -> Self {
        Self {
            _padding: [0; 64],
            data: [NormalTrb { data_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
            enqueue: AtomicU32::new(0),
            dequeue: AtomicU32::new(0),
            cycle_state: AtomicU32::new(1),
        }
    }

    /// Get the physical base address of the TRB data
    /// This is the address that should be programmed into CRCR
    pub fn trb_base(&self) -> u64 {
        &self.data[0] as *const NormalTrb as u64
    }

    /// Verify alignment (for debugging)
    pub fn is_aligned(&self) -> bool {
        self.trb_base() & 0x3F == 0
    }

    /// Enqueue a TRB
    pub unsafe fn enqueue(&self, trb: &NormalTrb) -> Result<(), UsbError> {
        let index = self.enqueue.load(Ordering::Acquire) as usize;
        let cycle = self.cycle_state.load(Ordering::Acquire);

        // Check if ring is full
        let next_index = (index + 1) % TRB_RING_SIZE;
        if next_index == self.dequeue.load(Ordering::Acquire) as usize {
            return Err(UsbError::Timeout);
        }

        // Write TRB
        let trb_ptr = &self.data[index] as *const NormalTrb as *mut NormalTrb;
        (*trb_ptr).data_ptr = trb.data_ptr;
        (*trb_ptr).status = trb.status;
        (*trb_ptr).control = trb.control | (cycle & 1);

        // Update enqueue pointer
        self.enqueue.store(next_index as u32, Ordering::Release);

        // Toggle cycle bit if wrapping
        if next_index == 0 {
            let new_cycle = cycle ^ 1;
            self.cycle_state.store(new_cycle, Ordering::Release);
        }

        Ok(())
    }

    /// Get current cycle state
    pub fn cycle(&self) -> u64 {
        self.cycle_state.load(Ordering::Acquire) as u64
    }
}

/// Event Ring Buffer (must be 64-byte aligned)
#[repr(C, align(64))]
pub struct AlignedEventRing {
    /// Padding to force alignment
    _padding: [u8; 64],
    /// TRB data
    pub data: [EventTrb; TRB_RING_SIZE],
    /// Dequeue pointer
    pub dequeue: AtomicU32,
    /// Cycle state
    pub cycle_state: AtomicU32,
}

impl AlignedEventRing {
    /// Create a new aligned event ring
    pub const fn new() -> Self {
        Self {
            _padding: [0; 64],
            data: [EventTrb { trb_ptr: 0, status: 0, control: 0 }; TRB_RING_SIZE],
            dequeue: AtomicU32::new(0),
            cycle_state: AtomicU32::new(1),
        }
    }

    /// Get the physical base address of the TRB data
    pub fn trb_base(&self) -> u64 {
        &self.data[0] as *const EventTrb as u64
    }

    /// Verify alignment
    pub fn is_aligned(&self) -> bool {
        self.trb_base() & 0x3F == 0
    }

    /// Dequeue an event TRB
    pub unsafe fn dequeue(&self) -> Option<EventTrb> {
        let index = self.dequeue.load(Ordering::Acquire) as usize;
        let cycle = self.cycle_state.load(Ordering::Acquire);

        let trb = self.data[index];
        let trb_cycle = (trb.control & TRB_CYCLE_BIT) as u32;

        // Check if TRB is owned by controller
        if trb_cycle != (cycle & 1) {
            return None;
        }

        // Update dequeue pointer
        let next_index = (index + 1) % TRB_RING_SIZE;
        self.dequeue.store(next_index as u32, Ordering::Release);

        // Toggle cycle bit if wrapping
        if next_index == 0 {
            let new_cycle = cycle ^ 1;
            self.cycle_state.store(new_cycle, Ordering::Release);
        }

        Some(trb)
    }

    /// Get dequeue pointer for ERDP
    pub fn dequeue_ptr(&self) -> u64 {
        let base = self.trb_base();
        let offset = self.dequeue.load(Ordering::Acquire) as u64 * 16;
        base + offset
    }
}

/// ERST Entry (per xHCI spec)
#[repr(C)]
pub struct ErstEntry {
    pub segment_base: u64,
    pub segment_size: u32,
    _reserved: u32,
}

/// Event Ring Segment Table (must be 64-byte aligned)
#[repr(C, align(64))]
pub struct AlignedErst {
    /// Padding to force alignment
    _padding: [u8; 64],
    /// ERST entries
    pub entries: [ErstEntry; 1],  // Single segment for now
}

impl AlignedErst {
    /// Create a new ERST
    pub const fn new() -> Self {
        Self {
            _padding: [0; 64],
            entries: [ErstEntry {
                segment_base: 0,
                segment_size: TRB_RING_SIZE as u32,
                _reserved: 0,
            }],
        }
    }

    /// Get physical address
    pub fn base(&self) -> u64 {
        &self.entries[0] as *const ErstEntry as u64
    }

    /// Verify alignment
    pub fn is_aligned(&self) -> bool {
        self.base() & 0x3F == 0
    }

    /// Initialize with event ring address
    pub unsafe fn init(&mut self, event_ring: &AlignedEventRing) {
        self.entries[0].segment_base = event_ring.trb_base();
        self.entries[0].segment_size = TRB_RING_SIZE as u32;
    }
}

/// DCBAA - Device Context Base Address Array (must be 64-byte aligned)
/// 256 entries of 64-bit pointers
#[repr(C, align(64))]
pub struct AlignedDcbaa {
    /// Padding to force alignment
    _padding: [u8; 64],
    /// DCBAA data (256 entries)
    pub data: [u64; 256],
}

impl AlignedDcbaa {
    /// Create a new DCBAA
    pub const fn new() -> Self {
        Self {
            _padding: [0; 64],
            data: [0; 256],
        }
    }

    /// Get physical address
    pub fn base(&self) -> u64 {
        &self.data[0] as *const u64 as u64
    }

    /// Verify alignment
    pub fn is_aligned(&self) -> bool {
        self.base() & 0x3F == 0
    }
}

/// ============================================================
/// GLOBAL ALLOCATIONS
/// ============================================================
/// These are the actual DMA buffers used by xHCI.
/// They are static and placed by the linker, but the
/// internal padding ensures the data fields are aligned.

pub static mut COMMAND_RING: AlignedCommandRing = AlignedCommandRing::new();
pub static mut EVENT_RING: AlignedEventRing = AlignedEventRing::new();
pub static mut ERST: AlignedErst = AlignedErst::new();
pub static mut DCBAA: AlignedDcbaa = AlignedDcbaa::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alignments() {
        unsafe {
            assert!(COMMAND_RING.is_aligned());
            assert!(EVENT_RING.is_aligned());
            assert!(ERST.is_aligned());
            assert!(DCBAA.is_aligned());
        }
    }

    #[test]
    fn test_ring_sizes() {
        assert_eq!(TRB_RING_SIZE, 16);
        assert_eq!(core::mem::size_of::<NormalTrb>(), 16);
        assert_eq!(core::mem::size_of::<EventTrb>(), 16);
    }
}
