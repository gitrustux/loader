// Copyright 2025 The Rustux Authors
//
// xHCI Transfer Request Blocks (TRBs)
//
// This module defines the TRB structures used by xHCI for USB transfers.

/// TRB Template field (bits [55:48] of Parameter 0)
/// Used to specify what type of TRB this is
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum TrbType {
    Normal = 1,
    SetupStage = 2,
    DataStage = 3,
    StatusStage = 4,
    Isoch = 5,
    Link = 6,
    EventData = 7,
    NoOp = 8,
    EnableSlot = 9,
    DisableSlot = 10,
    AddressDevice = 11,
    ConfigureEndpoint = 12,
    EvaluateContext = 13,
    ResetEndpoint = 14,
    StopEndpoint = 15,
    SetDequeuePointer = 16,
    ResetDevice = 17,
    ForceEvent = 18,
    NegotiateBandwidth = 19,
    SetLatencyToleranceValue = 20,
    GetPortBandwidth = 21,
    ForceHeader = 22,
    NoOpCommand = 23,
    GetExtendedProperty = 24,
    SetExtendedProperty = 25,
    // Transfer Event TRB = 32
    // Command Completion Event = 33
    TransferEvent = 32,
    CommandCompletion = 33,
    PortStatusChangeEvent = 34,
    BandwidthRequest = 35,
    DoorbellEvent = 36,
    HostControllerEvent = 37,
    DeviceNotificationEvent = 38,
    MfindexWrapEvent = 39,
}

/// TRB Common fields (first 8 bytes of all TRBs)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct TrbCommon {
    /// Parameter 0 (data pointer or slot/context info)
    pub param0: u64,
    /// Parameter 1 (status + TRB Type)
    pub param1: u32,
    /// Control bits (cycle bit, IOC, etc)
    pub control: u32,
}

/// Normal TRB for bulk/interrupt transfers
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct NormalTrb {
    /// Data buffer pointer (64-bit)
    pub data_ptr: u64,
    /// TRB Length + TD Size + Interrupter Target + Interrupt On Complete
    pub status: u32,
    /// TRB Type (1) + Direction + Chain + Toggle Cycle + IOC
    pub control: u32,
}

/// Setup Stage TRB for control transfers
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct SetupTrb {
    /// Request type + Request + Value + Index (USB setup packet)
    pub request: u64,
    /// Length + TRB Type (2) + Direction + Interrupt On Complete
    pub status: u32,
    /// TRB Type (2) + Stage + Cycle Bit
    pub control: u32,
}

/// Status Stage TRB for control transfers
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct StatusTrb {
    /// Reserved
    pub reserved: u64,
    /// TRB Length + TRB Type (4) + Direction + Interrupt On Complete
    pub status: u32,
    /// TRB Type (4) + Direction + Cycle Bit
    pub control: u32,
}

/// Link TRB for ring segmentation
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct LinkTrb {
    /// Ring segment pointer
    pub ptr: u64,
    /// Reserved + Interrupter Target
    pub status: u32,
    /// TRB Type (6) + Toggle Cycle + Cycle Bit
    pub control: u32,
}

/// Transfer Event TRB (in event ring)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct EventTrb {
    /// TRB Pointer (points to TRB that generated event)
    pub trb_ptr: u64,
    /// TRB Transfer Length + Completion Code + TD Remainder
    pub status: u32,
    /// TRB Type (32) + Slot ID + Endpoint ID
    pub control: u32,
}

/// TRB Completion Codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompletionCode {
    Success = 1,
    TrbError = 2,
    Stall = 3,
    ResourceError = 4,
    BandwidthError = 5,
    NoSlotsError = 6,
    InvalidStreamTypeError = 7,
    SlotNotEnabledError = 8,
    EpNotEnabledError = 9,
    ShortPacket = 13,
    RingUnderrun = 14,
    RingOverrun = 15,
    VfEventRingFullError = 16,
    ParameterError = 17,
    BandwidthOverrunError = 18,
    ContextStateError = 19,
    NoPingResponseError = 20,
    EventRingFullError = 21,
    IncompatibleDeviceError = 22,
    MissedServiceError = 23,
    CommandRingStopped = 24,
    CommandAborted = 25,
    Stopped = 26,
    StoppedLengthInvalid = 27,
    StoppedShortPacket = 28,
    MaxExitLatencyTooLarge = 29,
    IsochBufferOverrun = 31,
    EventLostError = 32,
    UndefinedError = 33,
    InvalidStreamIdError = 34,
    SecondaryBandwidthError = 35,
    SplitTransactionError = 36,
}

impl From<u8> for CompletionCode {
    fn from(value: u8) -> Self {
        match value {
            1 => CompletionCode::Success,
            2 => CompletionCode::TrbError,
            3 => CompletionCode::Stall,
            4 => CompletionCode::ResourceError,
            5 => CompletionCode::BandwidthError,
            6 => CompletionCode::NoSlotsError,
            7 => CompletionCode::InvalidStreamTypeError,
            8 => CompletionCode::SlotNotEnabledError,
            9 => CompletionCode::EpNotEnabledError,
            13 => CompletionCode::ShortPacket,
            14 => CompletionCode::RingUnderrun,
            15 => CompletionCode::RingOverrun,
            16 => CompletionCode::VfEventRingFullError,
            17 => CompletionCode::ParameterError,
            18 => CompletionCode::BandwidthOverrunError,
            19 => CompletionCode::ContextStateError,
            20 => CompletionCode::NoPingResponseError,
            21 => CompletionCode::EventRingFullError,
            22 => CompletionCode::IncompatibleDeviceError,
            23 => CompletionCode::MissedServiceError,
            24 => CompletionCode::CommandRingStopped,
            25 => CompletionCode::CommandAborted,
            26 => CompletionCode::Stopped,
            27 => CompletionCode::StoppedLengthInvalid,
            28 => CompletionCode::StoppedShortPacket,
            29 => CompletionCode::MaxExitLatencyTooLarge,
            31 => CompletionCode::IsochBufferOverrun,
            32 => CompletionCode::EventLostError,
            33 => CompletionCode::UndefinedError,
            34 => CompletionCode::InvalidStreamIdError,
            35 => CompletionCode::SecondaryBandwidthError,
            36 => CompletionCode::SplitTransactionError,
            _ => CompletionCode::TrbError,
        }
    }
}

/// TRB Control bits
pub const TRB_CYCLE_BIT: u32 = 1 << 0;
pub const TRB_IOC: u32 = 1 << 5;  // Interrupt On Completion
pub const TRB_CHAIN_BIT: u32 = 1 << 4;
pub const TRB_TOGGLE_CYCLE_BIT: u32 = 1 << 1;
pub const TRB_ENT_BULK_OUT: u32 = 1 << 16;  // Endpoint Type
pub const TRB_ENT_BULK_IN: u32 = 2 << 16;
pub const TRB_ENT_INTERRUPT_IN: u32 = 3 << 16;

/// Ring size (power of 2)
pub const TRB_RING_SIZE: usize = 16;

/// Get TRB type from control field
pub fn trb_type(control: u32) -> TrbType {
    let type_val = ((control >> 10) & 0x3F) as u8;
    match type_val {
        1 => TrbType::Normal,
        2 => TrbType::SetupStage,
        3 => TrbType::DataStage,
        4 => TrbType::StatusStage,
        5 => TrbType::Isoch,
        6 => TrbType::Link,
        7 => TrbType::EventData,
        8 => TrbType::NoOp,
        32 => TrbType::TransferEvent,
        33 => TrbType::CommandCompletion,
        _ => TrbType::NoOp,
    }
}

/// Get completion code from status field
pub fn completion_code(status: u32) -> CompletionCode {
    let code = (status >> 24) as u8;
    CompletionCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trb_sizes() {
        assert_eq!(core::mem::size_of::<NormalTrb>(), 16);
        assert_eq!(core::mem::size_of::<SetupTrb>(), 16);
        assert_eq!(core::mem::size_of::<StatusTrb>(), 16);
        assert_eq!(core::mem::size_of::<LinkTrb>(), 16);
        assert_eq!(core::mem::size_of::<EventTrb>(), 16);
    }
}
