//! ACPI table parsing for interrupt routing
//!
//! This module reads ACPI tables before exiting boot services to determine
//! the correct interrupt routing configuration. Specifically, we need to
//! check the MADT (Multiple APIC Description Table) for interrupt source
//! overrides that may remap IRQ1 (keyboard) to a different GSI.

/// ACPI 1.0 RSDP (Root System Description Pointer)
#[repr(C, packed)]
pub struct Rsdp {
    pub signature: [u8; 8],      // "RSD PTR "
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
}

/// ACPI 2.0+ RSDP (Extended)
#[repr(C, packed)]
pub struct RsdpExtended {
    pub base: Rsdp,
    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

/// SDT (System Description Table) header
#[repr(C, packed)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

/// MADT signature
const MADT_SIGNATURE: &[u8; 4] = b"APIC";

/// MADT table structure
#[repr(C, packed)]
pub struct Madt {
    pub header: SdtHeader,
    pub local_apic_address: u32,
    pub flags: u32,
}

/// MADT entry header
#[repr(C, packed)]
pub struct MadtEntryHeader {
    pub entry_type: u8,
    pub length: u8,
}

/// Interrupt Source Override entry (Type 2)
///
/// This is the critical structure for fixing keyboard IRQ routing.
/// Many UEFI systems override the default ISA IRQ mapping.
#[repr(C, packed)]
pub struct InterruptSourceOverrideEntry {
    pub header: MadtEntryHeader,
    pub bus: u8,           // 0 = ISA
    pub source_irq: u8,    // Source IRQ (e.g., 1 for keyboard)
    pub gsi: u32,          // Global System Interrupt (what we actually use)
    pub flags: u16,        // Polarity and trigger mode
}

/// Interrupt source override information for IRQ1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Irq1Override {
    /// The GSI that IRQ1 is actually mapped to
    pub gsi: u32,
    /// Polarity: 0 = active high, 1 = active low
    pub active_low: bool,
    /// Trigger mode: 0 = edge, 1 = level
    pub level_triggered: bool,
}

impl Irq1Override {
    /// Default configuration when no override is present
    pub const DEFAULT: Irq1Override = Irq1Override {
        gsi: 1,
        active_low: false,
        level_triggered: false,
    };
}

/// Find RSDP from UEFI configuration tables
///
/// This must be called BEFORE exiting boot services.
pub unsafe fn find_rsdp() -> Option<u64> {
    use uefi::table::system_table_raw;
    use uefi_raw::table::system::SystemTable;
    use uefi::table::cfg;

    // Get the system table using uefi crate's helper
    let st = system_table_raw()?;

    let system_table: &SystemTable = st.as_ref();

    // ACPI 2.0 GUID
    let acpi2_guid = cfg::ConfigTableEntry::ACPI2_GUID;

    // Search configuration tables for ACPI 2.0 GUID
    for i in 0..system_table.number_of_configuration_table_entries {
        let entry_ptr = system_table.configuration_table.add(i);
        let entry = &*entry_ptr;

        if entry.vendor_guid == acpi2_guid && !entry.vendor_table.is_null() {
            return Some(entry.vendor_table as u64);
        }
    }

    None
}

/// Verify RSDP checksum
fn verify_rsdp_checksum(rsdp: &Rsdp) -> bool {
    let ptr = rsdp as *const Rsdp as *const u8;
    let len = core::mem::size_of::<Rsdp>();
    let mut sum: u8 = 0;

    for i in 0..len {
        unsafe {
            sum = sum.wrapping_add(*ptr.add(i));
        }
    }

    sum == 0
}

/// Parse MADT and find IRQ1 override
///
/// Returns the interrupt override information for IRQ1 (keyboard).
/// If no override is present, returns the default configuration (GSI=1, edge, active-high).
pub unsafe fn find_irq1_override(rsdp_address: u64) -> Irq1Override {
    // Read RSDP
    let rsdp = &*(rsdp_address as *const Rsdp);

    if !verify_rsdp_checksum(rsdp) {
        // Invalid checksum, use default
        return Irq1Override::DEFAULT;
    }

    // Determine if we have XSDT (ACPI 2.0+) or RSDT (ACPI 1.0)
    let has_xsdt = rsdp.revision >= 2;

    // Get the root table address
    let root_table_address = if has_xsdt {
        let rsdp_ext = &*(rsdp_address as *const RsdpExtended);
        rsdp_ext.xsdt_address
    } else {
        rsdp.rsdt_address as u64
    };

    if root_table_address == 0 {
        return Irq1Override::DEFAULT;
    }

    // Search for MADT in the root table
    let header = &*(root_table_address as *const SdtHeader);
    let entry_size = if has_xsdt { 8 } else { 4 }; // XSDT uses 64-bit pointers, RSDT uses 32-bit

    let num_entries = (header.length as usize - core::mem::size_of::<SdtHeader>()) / entry_size;

    for i in 0..num_entries {
        let entry_ptr = if has_xsdt {
            let ptr_array = (root_table_address + core::mem::size_of::<SdtHeader>() as u64) as *const u64;
            *ptr_array.add(i)
        } else {
            let ptr_array = (root_table_address + core::mem::size_of::<SdtHeader>() as u64) as *const u32;
            *ptr_array.add(i) as u64
        };

        if entry_ptr == 0 {
            continue;
        }

        let entry_header = &*(entry_ptr as *const SdtHeader);

        // Check if this is the MADT
        if &entry_header.signature == MADT_SIGNATURE {
            // Found MADT, parse it
            return parse_madt_for_irq1(entry_ptr);
        }
    }

    // MADT not found, use default
    Irq1Override::DEFAULT
}

/// Parse MADT table to find IRQ1 interrupt source override
unsafe fn parse_madt_for_irq1(madt_address: u64) -> Irq1Override {
    let madt = &*(madt_address as *const Madt);

    // Entries start after the MADT header
    let header_size = core::mem::size_of::<SdtHeader>() + 8; // SDTHeader + local_apic_address + flags
    let mut offset = header_size;

    while offset < madt.header.length as usize {
        let entry_ptr = (madt_address as *const u8).add(offset) as *const MadtEntryHeader;
        let entry_header = &*entry_ptr;

        // Check if this is an interrupt source override (type 2)
        if entry_header.entry_type == 2 {
            let override_entry = &*(entry_ptr as *const InterruptSourceOverrideEntry);

            // Check if this is the IRQ1 (keyboard) override
            if override_entry.bus == 0 && override_entry.source_irq == 1 {
                // Found IRQ1 override!
                let flags = override_entry.flags;
                let active_low = (flags & 0x0002) != 0;  // Bit 1: polarity (0 = active high, 1 = active low)
                let level_triggered = (flags & 0x0008) != 0;  // Bit 3: trigger mode (0 = edge, 1 = level)

                return Irq1Override {
                    gsi: override_entry.gsi,
                    active_low,
                    level_triggered,
                };
            }
        }

        // Move to next entry
        offset += entry_header.length as usize;
    }

    // No override for IRQ1, use default
    Irq1Override::DEFAULT
}

/// Debug function to print IRQ1 override information
pub fn debug_print_override(override_info: Irq1Override) {
    use crate::framebuffer;

    if !framebuffer::is_initialized() {
        return;
    }

    // Draw a pixel at position (4, 0) to indicate override status
    // Gray = default (no override), Orange = override present
    let (r, g, b) = if override_info == Irq1Override::DEFAULT {
        (0x62, 0x62, 0x62) // Gray - no override
    } else {
        (0xFF, 0xA6, 0x00) // Orange - override present
    };
    unsafe { framebuffer::put_pixel(4, 0, r, g, b); }

    // Print information
    framebuffer::write_str("IRQ1 Override: ");
    if override_info == Irq1Override::DEFAULT {
        framebuffer::write_str("None (default GSI=1)\n");
    } else {
        framebuffer::write_str("GSI ");
        // Simple decimal digit (GSI is usually small, 0-23)
        if override_info.gsi < 10 {
            let digit = b'0' + (override_info.gsi as u8);
            let str_arr = [digit];
            let hex_str = core::str::from_utf8(&str_arr).unwrap_or("?");
            framebuffer::write_str(hex_str);
        } else {
            framebuffer::write_str(">");
        }
        framebuffer::write_str(" ");

        if override_info.active_low {
            framebuffer::write_str("ActiveLow ");
        } else {
            framebuffer::write_str("ActiveHigh ");
        }

        if override_info.level_triggered {
            framebuffer::write_str("Level\n");
        } else {
            framebuffer::write_str("Edge\n");
        }
    }
}
