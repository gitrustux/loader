// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Kernel Runtime - Post-ExitBootServices
//!
//! This module handles the kernel runtime after exiting UEFI boot services.

use alloc::vec::Vec;
use crate::filesystem::EmbeddedFileSystem;

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
    pub base_address: u64,
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
    /// Next PID to assign
    next_pid: u64,
}

impl KernelRuntime {
    /// Create a new kernel runtime instance
    pub fn new(memory_map: Vec<MemoryDescriptor>, map_size: usize, map_key: usize) -> Self {
        Self {
            memory_map,
            map_size,
            map_key,
            exited_boot_services: true,
            next_pid: 1,
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
    pub fn create_process(&mut self, name: &str, entry_point: u64, base_address: u64) -> Process {
        let pid = self.next_pid;
        self.next_pid += 1;

        Process {
            pid,
            name: alloc::string::String::from(name),
            state: ProcessState::Ready,
            entry_point,
            base_address,
        }
    }

    /// Load an ELF binary into memory with proper segment copying
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

        // Find the highest virtual address to determine memory needed
        let mut max_vaddr = 0u64;
        let mut min_vaddr = u64::MAX;

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
            let p_vaddr = u64::from_le_bytes(ph_data[16..24].try_into().unwrap());
            let p_memsz = u64::from_le_bytes(ph_data[40..48].try_into().unwrap());

            if p_type == PT_LOAD {
                if p_vaddr < min_vaddr {
                    min_vaddr = p_vaddr;
                }
                let segment_end = p_vaddr + p_memsz;
                if segment_end > max_vaddr {
                    max_vaddr = segment_end;
                }
            }
        }

        // Align base address
        let base_address = self.find_free_memory(256).ok_or("No free memory")?;
        let aligned_base = (base_address + 0xfff) & !0xfff;

        // Load each PT_LOAD segment
        for i in 0..e_phnum {
            let ph_offset = e_phoff + (i * e_phentsize) as u64;
            let ph_data = &data[ph_offset as usize..ph_offset as usize + e_phentsize];

            let p_type = u32::from_le_bytes(ph_data[0..4].try_into().unwrap());
            let _p_flags = u32::from_le_bytes(ph_data[4..8].try_into().unwrap());
            let p_offset = u64::from_le_bytes(ph_data[8..16].try_into().unwrap());
            let p_vaddr = u64::from_le_bytes(ph_data[16..24].try_into().unwrap());
            let _p_paddr = u64::from_le_bytes(ph_data[24..32].try_into().unwrap());
            let p_filesz = u64::from_le_bytes(ph_data[32..40].try_into().unwrap());
            let p_memsz = u64::from_le_bytes(ph_data[40..48].try_into().unwrap());

            if p_type == PT_LOAD {
                let load_addr = aligned_base + p_vaddr;
                let file_end = p_offset + p_filesz;

                if file_end > data.len() as u64 {
                    return Err("Segment outside file");
                }

                // Copy segment data to memory
                if p_filesz > 0 {
                    unsafe {
                        let dst = load_addr as *mut u8;
                        let src = data[p_offset as usize..file_end as usize].as_ptr();
                        core::ptr::copy_nonoverlapping(src, dst, p_filesz as usize);
                    }
                }

                // Zero BSS
                if p_memsz > p_filesz {
                    unsafe {
                        let bss_start = load_addr + p_filesz;
                        let bss_size = (p_memsz - p_filesz) as usize;
                        core::ptr::write_bytes(bss_start as *mut u8, 0, bss_size);
                    }
                }
            }
        }

        Ok(LoadedBinary {
            entry_point: aligned_base + e_entry,
            base_address: aligned_base,
            size: max_vaddr - min_vaddr,
        })
    }

    /// Execute a loaded binary via context switch
    pub fn execute_binary(&self, binary: &LoadedBinary) -> Result<(), &'static str> {
        // Flush instruction cache
        unsafe {
            // TODO: Add proper cache flush for x86_64
            // For x86_64, we may need to use wbinvd or similar

            // Create function pointer to entry point
            let entry: extern "C" fn() = core::mem::transmute(binary.entry_point);

            // Jump to entry point
            // This will not return in a real implementation
            // For now, we simulate execution
            let _ = entry;

            Ok(())
        }
    }

    /// Execute a binary by path using embedded filesystem
    pub fn execute(&mut self, path: &str) -> Result<(), &'static str> {
        // Get the embedded filesystem
        let filesystem = unsafe {
            crate::filesystem::get_filesystem()
                .ok_or("Filesystem not initialized")?
        };

        // Read the binary
        let file = filesystem.read(path)
            .ok_or("File not found")?;

        if !file.is_executable {
            return Err("File is not executable");
        }

        // Load the ELF binary
        let binary = self.load_elf(&file.data)?;

        // Create process entry
        let process = self.create_process(path, binary.entry_point, binary.base_address);

        // Execute via context switch
        // Note: In a real implementation, this would not return
        let _ = process;
        self.execute_binary(&binary)?;

        Ok(())
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
