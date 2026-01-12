// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Kernel Runtime - Post-ExitBootServices
//!
//! This module handles the kernel runtime after exiting UEFI boot services.

use alloc::vec::Vec;

/// Memory descriptor for kernel runtime (matches UEFI memory descriptor)
#[derive(Debug, Clone, Copy)]
pub struct MemoryDescriptor {
    pub physical_start: u64,
    pub number_of_pages: u64,
    pub memory_type: u32,
    pub attribute: u64,
}

/// Simple process/task abstraction
#[derive(Debug)]
pub struct Process {
    pub pid: u64,
    pub name: alloc::string::String,
    pub state: ProcessState,
    pub entry_point: u64,
}

/// Process state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Sleeping,
    Zombie,
}

/// ELF header information (minimal parsing)
#[repr(C)]
#[derive(Debug)]
pub struct ElfHeader {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF program header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ElfProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// Program header types
pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_SHLIB: u32 = 5;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;

/// Program header flags
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// Loaded binary information
#[derive(Debug)]
pub struct LoadedBinary {
    pub entry_point: u64,
    pub base_address: u64,
    pub size: u64,
}

/// Kernel runtime state
pub struct KernelRuntime {
    /// Memory map captured before ExitBootServices
    pub memory_map: Vec<MemoryDescriptor>,
    /// Map size for future GetMemoryMap calls
    pub map_size: usize,
    /// Map key for ExitBootServices
    pub map_key: usize,
    /// Flag indicating if ExitBootServices has been called
    pub exited_boot_services: bool,
}

impl KernelRuntime {
    /// Create a new kernel runtime instance
    pub fn new(memory_map: Vec<MemoryDescriptor>, map_size: usize, map_key: usize) -> Self {
        Self {
            memory_map,
            map_size,
            map_key,
            exited_boot_services: true,
        }
    }

    /// Check if we're in runtime mode (boot services exited)
    pub fn is_runtime(&self) -> bool {
        self.exited_boot_services
    }

    /// Find available memory pages for allocation
    pub fn find_free_memory(&self, pages: u64) -> Option<u64> {
        for desc in &self.memory_map {
            // Conventional memory (type 7) that's large enough
            if desc.memory_type == 7 && desc.number_of_pages >= pages {
                return Some(desc.physical_start);
            }
        }
        None
    }

    /// Create a new process
    pub fn create_process(&mut self, name: &str, entry_point: u64) -> Process {
        Process {
            pid: self.memory_map.len() as u64,
            name: alloc::string::String::from(name),
            state: ProcessState::Ready,
            entry_point,
        }
    }

    /// Load an ELF binary into memory
    pub fn load_elf(&self, data: &[u8]) -> Result<LoadedBinary, &'static str> {
        // Validate ELF magic
        if data.len() < 64 || &data[0..4] != b"\x7fELF" {
            return Err("Invalid ELF magic");
        }

        // Check if it's 64-bit
        if data[4] != 2 {
            return Err("Not a 64-bit ELF");
        }

        // Check if it's little-endian
        if data[5] != 1 {
            return Err("Not little-endian");
        }

        // Parse ELF header
        let e_entry = u64::from_le_bytes(data[24..32].try_into().unwrap());
        let e_phoff = u64::from_le_bytes(data[32..40].try_into().unwrap());
        let e_phentsize = u16::from_le_bytes(data[54..56].try_into().unwrap()) as usize;
        let e_phnum = u16::from_le_bytes(data[56..58].try_into().unwrap()) as usize;

        // Find a location to load the binary
        let base_address = self.find_free_memory(256).ok_or("No free memory")?;

        // Load program headers
        for i in 0..e_phnum {
            let ph_offset = e_phoff + (i * e_phentsize) as u64;
            if ph_offset + e_phentsize as u64 > data.len() as u64 {
                return Err("Program header outside file");
            }

            let ph_data = &data[ph_offset as usize..ph_offset as usize + e_phentsize];
            if ph_data.len() < 56 {
                return Err("Invalid program header");
            }

            let p_type = u32::from_le_bytes(ph_data[0..4].try_into().unwrap());
            let p_flags = u32::from_le_bytes(ph_data[4..8].try_into().unwrap());
            let p_offset = u64::from_le_bytes(ph_data[8..16].try_into().unwrap());
            let p_vaddr = u64::from_le_bytes(ph_data[16..24].try_into().unwrap());
            let p_paddr = u64::from_le_bytes(ph_data[24..32].try_into().unwrap());
            let p_filesz = u64::from_le_bytes(ph_data[32..40].try_into().unwrap());
            let p_memsz = u64::from_le_bytes(ph_data[40..48].try_into().unwrap());

            if p_type == PT_LOAD {
                // Load this segment into memory
                let load_addr = base_address + p_vaddr;
                let file_end = p_offset + p_filesz;

                if file_end > data.len() as u64 {
                    return Err("Segment outside file");
                }

                // TODO: Actually copy the segment to memory
                // For now, just validate the segment
                if p_filesz > 0 {
                    // Segment would be loaded at load_addr
                }

                // Zero BSS
                if p_memsz > p_filesz {
                    // BSS would be zeroed
                }
            }
        }

        Ok(LoadedBinary {
            entry_point: base_address + e_entry,
            base_address,
            size: 256 * 4096, // 256 pages
        })
    }

    /// Execute a loaded binary
    pub fn execute_binary(&self, binary: &LoadedBinary) -> Result<(), &'static str> {
        let _ = binary;
        Err("Process execution not yet implemented - needs context switch")
    }

    /// Execute a binary by path (stub for future filesystem support)
    pub fn execute(&mut self, path: &str) -> Result<(), &'static str> {
        // For now, this is a stub that indicates filesystem is needed
        let _ = path;
        Err("Filesystem access not yet implemented - external apps cannot be loaded from disk")
    }
}

/// Global runtime state (initialized after ExitBootServices)
static mut KERNEL_RUNTIME: Option<KernelRuntime> = None;

/// Initialize the kernel runtime (called after ExitBootServices)
pub unsafe fn init_runtime(
    memory_map: Vec<MemoryDescriptor>,
    map_size: usize,
    map_key: usize,
) {
    KERNEL_RUNTIME = Some(KernelRuntime::new(memory_map, map_size, map_key));
}

/// Get the global runtime instance
pub unsafe fn get_runtime() -> Option<&'static mut KernelRuntime> {
    KERNEL_RUNTIME.as_mut()
}

/// Check if we're in runtime mode
pub fn is_runtime_mode() -> bool {
    unsafe { KERNEL_RUNTIME.is_some() }
}
