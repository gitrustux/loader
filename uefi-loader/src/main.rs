#![no_std]
#![no_main]

extern crate alloc;

use uefi::prelude::*;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::file::{File, FileAttribute, FileMode};
use uefi::proto::loaded_image::LoadedImage;
use uefi::table::cfg;
use uefi::table::system_table_raw;
use uefi::boot::{AllocateType, MemoryType};
use uefi::Status;
use alloc::vec::Vec;

// Global allocator for UEFI
#[global_allocator]
static ALLOCATOR: uefi::allocator::Allocator = uefi::allocator::Allocator;

// Required for UEFI no_std
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

// ============================================================================
// Memory Types - Adapted from Zircon's efi_memory_type conversion
// ============================================================================

/// Rustux memory type for kernel handoff
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustuxMemoryType {
    Available = 1,
    Reserved = 2,
    Reclaimable = 3,
    Peripheral = 4,
}

impl From<MemoryType> for RustuxMemoryType {
    fn from(efi_type: MemoryType) -> Self {
        match efi_type {
            MemoryType::LOADER_CODE
            | MemoryType::LOADER_DATA
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA
            | MemoryType::CONVENTIONAL => RustuxMemoryType::Available,

            MemoryType::MMIO
            | MemoryType::MMIO_PORT_SPACE => RustuxMemoryType::Peripheral,

            MemoryType::ACPI_RECLAIM
            | MemoryType::ACPI_NON_VOLATILE => RustuxMemoryType::Reclaimable,

            _ => RustuxMemoryType::Reserved,
        }
    }
}

/// Memory range descriptor for kernel handoff
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRange {
    pub base: u64,
    pub length: u64,
    pub mem_type: RustuxMemoryType,
}

/// Kernel handoff structure - passed to kernel on boot
#[repr(C)]
#[derive(Debug)]
pub struct KernelHandoff {
    pub memory_map: Vec<MemoryRange>,
    pub acpi_rsdp: Option<u64>,
    pub smbios_entry: Option<u64>,
    pub system_table: u64,
    pub framebuffer: Option<FramebufferInfo>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: FramebufferFormat,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum FramebufferFormat {
    RGB,
    BGR,
}

// ============================================================================
// UEFI Bootloader - Incorporating Zircon patterns
// ============================================================================

/// UEFI entry point
#[entry]
fn main() -> Status {
    // Initialize the system
    uefi::helpers::init().unwrap();

    uefi::system::with_stdout(|stdout| {
        stdout.clear().unwrap();
        stdout.enable_cursor(true).unwrap();

        // Simple ASCII banner
        stdout.output_string(cstr16!(
"==================================================================\r\n\
||                     RUSTUX BOOTLOADER                         ||\r\n\
||                   v0.3.0 - Zircon Patterns                   ||\r\n\
||==================================================================\r\n\
\r\n\
[Phase 1] UEFI Environment Initialization\r\n\
  - System table acquired\r\n\
  - Memory allocator initialized\r\n\
  - Console protocols ready\r\n\
\r\n\
[Phase 2] Platform Discovery (Zircon-inspired)\r\n\
"));

        // Zircon pattern: Discover ACPI tables
        match find_acpi_rsdp() {
            Some(_rsdp) => {
                let _ = stdout.output_string(cstr16!("  - ACPI RSDP: Found\r\n"));
            }
            None => {
                let _ = stdout.output_string(cstr16!("  - ACPI: Not present (warning)\r\n"));
            }
        }

        let _ = stdout.output_string(cstr16!("\r\n\
[Phase 3] Memory Map Acquisition\r\n\
"));

        // Get memory map using Zircon-inspired pattern
        match get_efi_memory_map(stdout) {
            Ok(memory_ranges) => {
                let _ = stdout.output_string(cstr16!("  - Memory map acquired\r\n"));
                let _ = stdout.output_string(cstr16!("    - Total ranges: "));
                // Simple count display
                let count = memory_ranges.len();
                if count >= 10 {
                    let _tens = count / 10;
                    let _ = stdout.output_string(cstr16!(">10\r\n"));
                } else {
                    let digits = [cstr16!("0\r\n"), cstr16!("1\r\n"), cstr16!("2\r\n"),
                                  cstr16!("3\r\n"), cstr16!("4\r\n"), cstr16!("5\r\n"),
                                  cstr16!("6\r\n"), cstr16!("7\r\n"), cstr16!("8\r\n"),
                                  cstr16!("9\r\n")];
                    if count > 0 && count <= 9 {
                        let _ = stdout.output_string(digits[count]);
                    }
                }
            }
            Err(_) => {
                let _ = stdout.output_string(cstr16!("  - Warning: Memory map acquisition failed\r\n"));
            }
        }

        let _ = stdout.output_string(cstr16!("\r\n\
[Phase 4] Kernel Loading\r\n\
  - Searching for /EFI/Rustux/kernel.efi\r\n\
"));

        match load_and_start_kernel() {
            Ok(_) => {
                // Should not reach here
                let _ = stdout.output_string(cstr16!("\r\n\
[ERROR] Kernel returned unexpectedly\r\n\
"));
                Status::ABORTED
            }
            Err(_) => {
                let _ = stdout.output_string(cstr16!("\r\n\
[ERROR] Kernel load failed\r\n\
"));
                Status::ABORTED
            }
        }
    });

    // Halt
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

// ============================================================================
// Memory Map Handling - Zircon-inspired implementation
// ============================================================================

/// Zircon-style GetMemoryMap implementation
/// Returns the memory map buffer, entry size, and map key for ExitBootServices
struct EfiMemoryMapInfo {
    buffer: *mut u8,
    size: usize,
    entry_size: usize,
    key: usize,
}

/// Get EFI memory map - Zircon pattern
/// First call gets size, second call fills the buffer
fn get_efi_memory_map_raw() -> Result<EfiMemoryMapInfo, uefi::Error> {
    // Zircon pattern: First call to get required buffer size
    let mut map_size = 0usize;
    let mut map_key = 0usize;
    let mut entry_size = 0usize;
    let mut entry_version = 0u32;

    let status = unsafe {
        let bt = uefi::table::system_table_raw()
            .ok_or(uefi::Status::NOT_FOUND)?;
        let st = bt.as_ref();
        let boot_services = st.boot_services;

        // GetMemoryMap first call - get buffer size
        // EFI_STATUS GetMemoryMap(
        //   IN OUT UINTN  *MemoryMapSize,
        //   OUT VOID *MemoryMap,
        //   OUT UINTN *MapKey,
        //   OUT UINTN *DescriptorSize,
        //   OUT UINT32 *DescriptorVersion
        // );
        let get_memory_map = (*boot_services).get_memory_map;
        get_memory_map(
            &mut map_size,
            core::ptr::null_mut(),
            &mut map_key,
            &mut entry_size,
            &mut entry_version,
        )
    };

    // EFI_BUFFER_TOO_SMALL (5) is expected on first call
    if !status.is_success() && status != Status::BUFFER_TOO_SMALL {
        return Err(uefi::Error::from(status));
    }

    // Zircon pattern: Add extra space for dynamic allocations
    // during ExitBootServices
    map_size += entry_size * 8;

    // Allocate buffer for memory map
    let buffer_pages = (map_size + 0xFFF) / 0x1000;
    let buffer = uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        buffer_pages,
    )?;

    let buffer_ptr = buffer.as_ptr() as *mut u8;

    // Get actual memory map
    let status = unsafe {
        let bt = uefi::table::system_table_raw()
            .ok_or(uefi::Status::NOT_FOUND)?;
        let st = bt.as_ref();
        let boot_services = st.boot_services;

        let get_memory_map = (*boot_services).get_memory_map;
        get_memory_map(
            &mut map_size,
            buffer_ptr as *mut uefi_raw::table::boot::MemoryDescriptor,
            &mut map_key,
            &mut entry_size,
            &mut entry_version,
        )
    };

    if !status.is_success() {
        return Err(uefi::Error::from(status));
    }

    Ok(EfiMemoryMapInfo {
        buffer: buffer_ptr,
        size: map_size,
        entry_size,
        key: map_key,
    })
}

/// Zircon-style memory range coalescing
/// Combine contiguous ranges of the same type
fn coalesce_ranges(ranges: &mut Vec<MemoryRange>) {
    if ranges.len() <= 1 {
        return;
    }

    // Zircon pattern: sort by physical address first
    ranges.sort_by_key(|r| r.base);

    let mut write_idx = 1;
    for read_idx in 1..ranges.len() {
        let prev = &ranges[write_idx - 1];
        let curr = &ranges[read_idx];

        // Check if ranges are contiguous and have same type
        if prev.mem_type == curr.mem_type && prev.base + prev.length == curr.base {
            // Merge into previous range
            ranges[write_idx - 1].length += curr.length;
        } else {
            // Keep this range separate
            if read_idx != write_idx {
                ranges[write_idx] = *curr;
            }
            write_idx += 1;
        }
    }

    ranges.truncate(write_idx);
}

/// Convert EFI memory map to Rustux format - Zircon pattern
fn get_efi_memory_map(_stdout: &mut uefi::proto::console::text::Output) -> Result<Vec<MemoryRange>, uefi::Error> {
    let map_info = get_efi_memory_map_raw()?;

    // Convert EFI memory descriptors to Rustux format
    let num_entries = map_info.size / map_info.entry_size;
    let mut ranges = Vec::new();

    for i in 0..num_entries {
        let desc_ptr = unsafe {
            (map_info.buffer as *const u8).add(i * map_info.entry_size)
                as *const uefi_raw::table::boot::MemoryDescriptor
        };
        let desc = unsafe { &*desc_ptr };

        // Zircon pattern: Ignore zero-length entries
        if desc.page_count > 0 {
            // Convert uefi_raw::MemoryType to uefi::MemoryType for our From impl
            let efi_memory_type: MemoryType = unsafe { core::mem::transmute(desc.ty) };
            let range = MemoryRange {
                base: desc.phys_start,
                length: desc.page_count * 4096, // UEFI page size
                mem_type: RustuxMemoryType::from(efi_memory_type),
            };
            ranges.push(range);
        }
    }

    // Zircon pattern: Coalesce contiguous ranges of same type
    coalesce_ranges(&mut ranges);

    Ok(ranges)
}

// ============================================================================
// ACPI Discovery - Zircon-inspired
// ============================================================================

/// ACPI RSDP (Root System Description Pointer) signature
const ACPI_RSDP_SIGNATURE: u64 = 0x2052545020445352; // "RSD PTR "

/// Find ACPI RSDP from UEFI configuration tables - Zircon pattern
fn find_acpi_rsdp() -> Option<u64> {
    // Zircon pattern: Search configuration tables for ACPI GUIDs
    // Try ACPI 2.0 GUID first, then ACPI 1.0 GUID
    let acpi2_guid = cfg::ConfigTableEntry::ACPI2_GUID;

    if let Some(st) = system_table_raw() {
        let system_table: &uefi_raw::table::system::SystemTable = unsafe { st.as_ref() };

        for i in 0..system_table.number_of_configuration_table_entries {
            let entry_ptr = unsafe {
                system_table.configuration_table.add(i)
            };
            let entry = unsafe { &*entry_ptr };

            if entry.vendor_guid == acpi2_guid && !entry.vendor_table.is_null() {
                // Found ACPI table
                let rsdp_ptr = entry.vendor_table as u64;
                return Some(rsdp_ptr);
            }
        }
    }

    None
}

// ============================================================================
// PE/COFF Format Definitions
// ============================================================================

/// PE/COFF header structures for manual image loading
#[repr(C)]
struct DosHeader {
    e_magic: u16,           // Magic number (0x5A4D = "MZ")
    e_cblp: u16,
    e_cp: u16,
    e_crlc: u16,
    e_cparhdr: u16,
    e_minalloc: u16,
    e_maxalloc: u16,
    e_ss: u16,
    e_sp: u16,
    e_csum: u16,
    e_ip: u16,
    e_cs: u16,
    e_lfarlc: u16,
    e_ovno: u16,
    e_res: [u16; 4],
    e_oemid: u16,
    e_oeminfo: u16,
    e_res2: [u16; 10],
    e_lfanew: u32,          // Offset to PE header
}

/// COFF File Header
#[repr(C)]
struct CoffFileHeader {
    machine: u16,
    number_of_sections: u16,
    time_date_stamp: u32,
    pointer_to_symbol_table: u32,
    number_of_symbols: u32,
    size_of_optional_header: u16,
    characteristics: u16,
}

/// PE Optional Header (PE32+ format)
#[repr(C)]
struct PeOptionalHeader {
    magic: u16,                     // 0x20b for PE32+
    major_linker_version: u8,
    minor_linker_version: u8,
    size_of_code: u32,
    size_of_initialized_data: u32,
    size_of_uninitialized_data: u32,
    address_of_entry_point: u32,
    base_of_code: u32,
    image_base: u64,
    section_alignment: u32,
    file_alignment: u32,
    major_os_version: u16,
    minor_os_version: u16,
    major_image_version: u16,
    minor_image_version: u16,
    major_subsystem_version: u16,
    minor_subsystem_version: u16,
    win32_version_value: u32,
    size_of_image: u32,
    size_of_headers: u32,
    check_sum: u32,
    subsystem: u16,
    dll_characteristics: u16,
    size_of_stack_reserve: u64,
    size_of_stack_commit: u64,
    size_of_heap_reserve: u64,
    size_of_heap_commit: u64,
    loader_flags: u32,
    number_of_rva_and_sizes: u32,
}

/// Data Directory entry
#[repr(C)]
struct DataDirectory {
    virtual_address: u32,
    size: u32,
}

/// Section header
#[repr(C)]
struct SectionHeader {
    name: [u8; 8],
    virtual_size: u32,
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
    pointer_to_relocations: u32,
    pointer_to_line_numbers: u32,
    number_of_relocations: u16,
    number_of_line_numbers: u16,
    characteristics: u32,
}

// ============================================================================
// Kernel Loading
// ============================================================================

/// Validate and display PE/COFF header information
fn validate_pe_coff(kernel_data: *const u8, file_size: usize) -> Result<(), &'static str> {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("  - Validating PE/COFF header...\r\n"));
    });

    // Check minimum size
    if file_size < 64 {
        return Err("File too small for DOS header");
    }

    // Parse DOS header
    let dos_header = unsafe { &*(kernel_data as *const DosHeader) };

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("    - DOS signature: "));
        if dos_header.e_magic == 0x5A4D {
            let _ = stdout.output_string(cstr16!("MZ (OK)\r\n"));
        } else {
            let _ = stdout.output_string(cstr16!("Invalid! (expected MZ)\r\n"));
        }
        let _ = stdout.output_string(cstr16!("    - PE offset: "));
        // Simple hex display for PE offset
        let pe_off = dos_header.e_lfanew;
        let hex = [cstr16!("0"), cstr16!("1"), cstr16!("2"), cstr16!("3"),
                   cstr16!("4"), cstr16!("5"), cstr16!("6"), cstr16!("7"),
                   cstr16!("8"), cstr16!("9"), cstr16!("A"), cstr16!("B"),
                   cstr16!("C"), cstr16!("D"), cstr16!("E"), cstr16!("F")];
        let _ = stdout.output_string(cstr16!("0x"));
        let _ = stdout.output_string(hex[((pe_off >> 28) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((pe_off >> 24) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((pe_off >> 20) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((pe_off >> 16) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((pe_off >> 12) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((pe_off >> 8) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((pe_off >> 4) & 0xF) as usize]);
        let _ = stdout.output_string(hex[(pe_off & 0xF) as usize]);
        let _ = stdout.output_string(cstr16!("\r\n"));
    });

    // Check PE offset is valid
    let pe_offset = dos_header.e_lfanew as usize;
    if pe_offset == 0 || pe_offset + 4 > file_size {
        return Err("Invalid PE offset");
    }

    // Check PE signature
    let pe_signature = unsafe { *(kernel_data.add(pe_offset) as *const u32) };
    if pe_signature != 0x00004550 {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.output_string(cstr16!("    - PE signature: Invalid! (expected PE00)\r\n"));
        });
        return Err("Invalid PE signature");
    }

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("    - PE signature: PE00 (OK)\r\n"));
    });

    // Parse COFF file header
    let coff_offset = pe_offset + 4;
    if coff_offset + core::mem::size_of::<CoffFileHeader>() > file_size {
        return Err("File too small for COFF header");
    }

    let coff_header = unsafe {
        &*(kernel_data.add(coff_offset) as *const CoffFileHeader)
    };

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("    - Machine type: "));
        // 0x8664 = AMD64/x86-64
        if coff_header.machine == 0x8664 {
            let _ = stdout.output_string(cstr16!("0x8664 (x86-64, OK)\r\n"));
        } else {
            let _ = stdout.output_string(cstr16!("Unknown (Not x86-64!)\r\n"));
        }
        let _ = stdout.output_string(cstr16!("    - Characteristics: "));
        if coff_header.characteristics & 0x0020 != 0 {
            let _ = stdout.output_string(cstr16!("EXECUTABLE_IMAGE\r\n"));
        } else {
            let _ = stdout.output_string(cstr16!("Unknown\r\n"));
        }
    });

    // Parse optional header
    let opt_header_offset = coff_offset + core::mem::size_of::<CoffFileHeader>();
    if opt_header_offset + core::mem::size_of::<PeOptionalHeader>() > file_size {
        return Err("File too small for optional header");
    }

    let opt_header = unsafe {
        &*(kernel_data.add(opt_header_offset) as *const PeOptionalHeader)
    };

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("    - PE Magic: "));
        // 0x20b = PE32+ (64-bit)
        if opt_header.magic == 0x020b {
            let _ = stdout.output_string(cstr16!("0x20b (PE32+, OK)\r\n"));
        } else {
            let _ = stdout.output_string(cstr16!("Unknown (Not PE32+!)\r\n"));
        }
        let _ = stdout.output_string(cstr16!("    - Subsystem: "));
        // 0x0a = EFI application
        if opt_header.subsystem == 0x0a {
            let _ = stdout.output_string(cstr16!("0x0a (EFI_APP, OK)\r\n"));
        } else if opt_header.subsystem == 0x0b {
            let _ = stdout.output_string(cstr16!("0x0b (EFI_BOOT_SERVICE_DRIVER)\r\n"));
        } else if opt_header.subsystem == 0x0c {
            let _ = stdout.output_string(cstr16!("0x0c (EFI_RUNTIME_DRIVER)\r\n"));
        } else {
            let _ = stdout.output_string(cstr16!("Unknown (Not EFI!)\r\n"));
        }
        let _ = stdout.output_string(cstr16!("    - Entry point RVA: "));
        let ep = opt_header.address_of_entry_point;
        let hex = [cstr16!("0"), cstr16!("1"), cstr16!("2"), cstr16!("3"),
                   cstr16!("4"), cstr16!("5"), cstr16!("6"), cstr16!("7"),
                   cstr16!("8"), cstr16!("9"), cstr16!("A"), cstr16!("B"),
                   cstr16!("C"), cstr16!("D"), cstr16!("E"), cstr16!("F")];
        let _ = stdout.output_string(cstr16!("0x"));
        let _ = stdout.output_string(hex[((ep >> 28) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((ep >> 24) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((ep >> 20) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((ep >> 16) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((ep >> 12) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((ep >> 8) & 0xF) as usize]);
        let _ = stdout.output_string(hex[((ep >> 4) & 0xF) as usize]);
        let _ = stdout.output_string(hex[(ep & 0xF) as usize]);
        let _ = stdout.output_string(cstr16!("\r\n"));
    });

    // Validate machine type
    if coff_header.machine != 0x8664 {
        return Err("Wrong machine type (not x86-64)");
    }

    // Validate PE32+ magic
    if opt_header.magic != 0x020b {
        return Err("Wrong PE magic (not PE32+)");
    }

    // Validate subsystem
    if opt_header.subsystem != 0x0a && opt_header.subsystem != 0x0b && opt_header.subsystem != 0x0c {
        return Err("Wrong subsystem (not EFI)");
    }

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("  - PE/COFF validation: PASSED\r\n"));
    });

    Ok(())
}

/// Reboot the system using UEFI runtime services
fn reboot_system() -> ! {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("\r\nRebooting system...\r\n"));
    });

    // Use runtime services to reset the system
    unsafe {
        if let Some(st) = uefi::table::system_table_raw() {
            let system_table = st.as_ref();
            let runtime_services = system_table.runtime_services;

            // reset_system is the correct field name
            let reset = (*runtime_services).reset_system;
            reset(
                uefi_raw::table::runtime::ResetType::COLD,
                uefi_raw::Status::SUCCESS,
                0,
                core::ptr::null_mut(),
            );
        }
    }

    // If reset failed, halt
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

/// Show error menu with reboot option
fn show_error_menu(_error_message: &str) -> ! {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("\r\n"));
        let _ = stdout.output_string(cstr16!("==================================================================\r\n"));
        let _ = stdout.output_string(cstr16!("||                    BOOT ERROR DETECTED                      ||\r\n"));
        let _ = stdout.output_string(cstr16!("==================================================================\r\n\r\n"));

        let _ = stdout.output_string(cstr16!("Boot failed. System will reboot in 5 seconds...\r\n"));
    });

    // Wait 5 seconds then reboot
    for _ in 0..50 {
        unsafe {
            let st = uefi::table::system_table_raw().unwrap();
            let system_table = st.as_ref();
            let boot_services = system_table.boot_services;
            let stall = (*boot_services).stall;
            stall(100000);  // 100ms * 50 = 5 seconds
        }
    }

    reboot_system();
}

/// Load and start the kernel.efi from disk
fn load_and_start_kernel() -> uefi::Result {
    // Get the loaded image protocol to find our device
    let image_handle = uefi::boot::image_handle();

    // Get the device handle
    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)?;
    let device = loaded_image.device().ok_or(uefi::Status::DEVICE_ERROR)?;

    // Get the SimpleFileSystem protocol
    let mut fs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device)?;

    // Open the volume
    let mut root = fs.open_volume()?;

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("  - Opened EFI volume\r\n"));
    });

    // Try to open kernel.efi
    let kernel_path = cstr16!("\\EFI\\Rustux\\kernel.efi");
    let kernel_file = root.open(kernel_path, FileMode::Read, FileAttribute::empty());

    match kernel_file {
        Ok(handle) => {
            uefi::system::with_stdout(|stdout| {
                let _ = stdout.output_string(cstr16!("  - Found kernel.efi, loading...\r\n"));
            });

            match handle.into_type().map_err(|e| e.status())? {
                uefi::proto::media::file::FileType::Regular(mut file) => {
                    // Read file info
                    let mut info_buf = [0u8; 256];
                    let info = file.get_info::<uefi::proto::media::file::FileInfo>(&mut info_buf)
                        .map_err(|e| uefi::Error::from(e.status()))?;

                    let file_size = info.file_size() as usize;

                    uefi::system::with_stdout(|stdout| {
                        let _ = stdout.output_string(cstr16!("  - Kernel file size: "));
                        // Simple size display - just show OK for now
                        let _ = stdout.output_string(cstr16!("OK\r\n"));
                    });

                    // Allocate memory for the kernel
                    let num_pages = (file_size + 0xFFF) / 0x1000;
                    let kernel_data = uefi::boot::allocate_pages(
                        AllocateType::AnyPages,
                        MemoryType::LOADER_DATA,
                        num_pages,
                    )?;

                    // Read the file
                    let kernel_slice = unsafe {
                        core::slice::from_raw_parts_mut(kernel_data.as_ptr(), file_size)
                    };
                    file.read(kernel_slice).map_err(|e| uefi::Error::from(e.status()))?;

                    uefi::system::with_stdout(|stdout| {
                        let _ = stdout.output_string(cstr16!("  - Kernel loaded into memory\r\n"));
                    });

                    // Validate PE/COFF header before calling LoadImage
                    let kernel_ptr = kernel_data.as_ptr() as *const u8;
                    if let Err(_e) = validate_pe_coff(kernel_ptr, file_size) {
                        uefi::system::with_stdout(|stdout| {
                            let _ = stdout.output_string(cstr16!("  - PE/COFF validation FAILED\r\n"));
                            let _ = stdout.output_string(cstr16!("  - Trying LoadImage anyway...\r\n"));
                        });
                    }

                    uefi::system::with_stdout(|stdout| {
                        let _ = stdout.output_string(cstr16!("  - Loading kernel image via EFI protocol...\r\n"));
                    });

                    // Load the kernel using the UEFI LoadImage service
                    // This properly handles PE relocation and entry point
                    let result = unsafe {
                        let bt = uefi::table::system_table_raw().unwrap();
                        let system_table = bt.as_ref();
                        let boot_services = system_table.boot_services;

                        // First, try to get the device path and use LoadImage with file path
                        // If that fails, load from memory buffer
                        let mut kernel_handle: *mut core::ffi::c_void = core::ptr::null_mut();
                        let load_image = (*boot_services).load_image;

                        uefi::system::with_stdout(|stdout| {
                            let _ = stdout.output_string(cstr16!("  - Calling LoadImage...\r\n"));
                        });

                        // Try loading from buffer (BootPolicy = FALSE)
                        let status = load_image(
                            false.into(),  // BootPolicy: FALSE = load from buffer
                            uefi::boot::image_handle().as_ptr(),
                            core::ptr::null(),  // No FilePath when loading from buffer
                            kernel_data.as_ptr() as *mut u8,
                            file_size,
                            &mut kernel_handle,
                        );

                        uefi::system::with_stdout(|stdout| {
                            let _ = stdout.output_string(cstr16!("  - LoadImage result: "));
                            if status.is_success() {
                                let _ = stdout.output_string(cstr16!("SUCCESS\r\n"));
                            } else {
                                let _ = stdout.output_string(cstr16!("FAILED (trying direct jump)...\r\n"));
                            }
                        });

                        if !status.is_success() {
                            // LoadImage failed, try direct entry point call
                            let dos_header = kernel_data.as_ptr() as *const u8;
                            let pe_offset = *(dos_header.add(0x3C) as *const u32) as usize;
                            let pe_header = dos_header.add(pe_offset);
                            let optional_header_offset = pe_offset + 0x18;
                            let entry_point_rva = *(pe_header.add(optional_header_offset + 0x10) as *const u32) as usize;
                            let image_base = kernel_data.as_ptr() as usize;
                            let entry_point = (image_base + entry_point_rva) as *const ();

                            uefi::system::with_stdout(|stdout| {
                                let _ = stdout.output_string(cstr16!("  - Direct jump to 0x"));
                                let addr = entry_point as usize;
                                let _ = stdout.output_string(cstr16!("XXXX\r\n"));
                                let _ = stdout.output_string(cstr16!("  - Starting kernel...\r\n"));
                            });

                            // UEFI entry point signature
                            type EfiEntry = extern "efiapi" fn(*mut core::ffi::c_void, *mut uefi_raw::table::system::SystemTable) -> Status;
                            let efi_entry: EfiEntry = core::mem::transmute(entry_point);

                            let entry_status = efi_entry(
                                uefi::boot::image_handle().as_ptr(),
                                system_table as *const _ as *mut _
                            );

                            uefi::system::with_stdout(|stdout| {
                                let _ = stdout.output_string(cstr16!("  - Kernel returned (shouldn't happen)\r\n"));
                            });

                            return Err(entry_status.into());
                        }

                        // LoadImage succeeded, now start the image
                        uefi::system::with_stdout(|stdout| {
                            let _ = stdout.output_string(cstr16!("  - Starting kernel image...\r\n"));
                        });

                        let start_image = (*boot_services).start_image;
                        let mut exit_data_size: usize = 0;
                        let mut exit_data: *mut u16 = core::ptr::null_mut();

                        let start_status = start_image(
                            kernel_handle,
                            &mut exit_data_size as *mut _,
                            &mut exit_data as *mut _
                        );

                        uefi::system::with_stdout(|stdout| {
                            let _ = stdout.output_string(cstr16!("  - Kernel returned (unexpected)!\r\n"));
                        });

                        if start_status.is_success() {
                            Err(uefi::Status::ABORTED)
                        } else {
                            Err(start_status)
                        }
                    };

                    result.map_err(|e| uefi::Error::from(e))
                }
                _ => {
                    show_error_menu("Error: kernel.efi is not a regular file");
                }
            }
        }
        Err(_e) => {
            show_error_menu("Error: kernel.efi not found at /EFI/Rustux/kernel.efi");
        }
    }
}
