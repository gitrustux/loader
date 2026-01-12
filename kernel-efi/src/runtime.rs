// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Kernel Runtime - Post-ExitBootServices Infrastructure
//!
//! This module provides the foundation for kernel runtime after exiting UEFI boot services.
//! It includes memory management, process execution, and ELF loading infrastructure.

use alloc::vec::Vec;

/// Memory descriptor for kernel runtime
#[derive(Debug, Clone, Copy)]
pub struct MemoryDescriptor {
    pub physical_start: u64,
    pub page_count: u64,
    pub memory_type: u32,  // MemoryType as u32 for compatibility
}

/// Simple process/task abstraction
#[derive(Debug)]
pub struct Process {
    pub pid: u64,
    pub name: alloc::string::String,
    pub state: ProcessState,
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
    pub e_ident: [u8; 16],  // ELF identification
    pub e_type: u16,        // Object file type
    pub e_machine: u16,     // Machine type
    pub e_version: u32,     // Object file version
    pub e_entry: u64,       // Entry point virtual address
    pub e_phoff: u64,       // Program header table file offset
    pub e_shoff: u64,       // Section header table file offset
    pub e_flags: u32,       // Processor-specific flags
    pub e_ehsize: u16,      // ELF header size
    pub e_phentsize: u16,   // Program header table entry size
    pub e_phnum: u16,       // Program header table entry count
    pub e_shentsize: u16,   // Section header table entry size
    pub e_shnum: u16,       // Section header table entry count
    pub e_shstrndx: u16,    // Section header string table index
}

/// ELF program header
#[repr(C)]
#[derive(Debug)]
pub struct ElfProgramHeader {
    pub p_type: u32,       // Segment type
    pub p_flags: u32,      // Segment flags
    pub p_offset: u64,     // Segment file offset
    pub p_vaddr: u64,      // Segment virtual address
    pub p_paddr: u64,      // Segment physical address
    pub p_filesz: u64,     // Segment size in file
    pub p_memsz: u64,      // Segment size in memory
    pub p_align: u64,      // Segment alignment
}

/// Kernel runtime state
pub struct KernelRuntime {
    /// Memory map captured before ExitBootServices
    pub memory_map: Vec<MemoryDescriptor>,
    /// Flag indicating if ExitBootServices has been called
    pub is_runtime: bool,
}

impl KernelRuntime {
    /// Create a new kernel runtime instance
    pub fn new(memory_map: Vec<MemoryDescriptor>) -> Self {
        Self {
            memory_map,
            is_runtime: true,
        }
    }

    /// Check if we're in runtime mode (boot services exited)
    pub fn is_runtime(&self) -> bool {
        self.is_runtime
    }

    /// Allocate memory from the captured memory map
    pub fn allocate_pages(&mut self, pages: u64) -> Option<u64> {
        // Simple allocation - find first available free pages
        for desc in &self.memory_map {
            if desc.memory_type == 7 && desc.page_count >= pages {  // 7 = CONVENTIONAL
                return Some(desc.physical_start);
            }
        }
        None
    }

    /// Create a new process
    pub fn create_process(&mut self, name: &str) -> Process {
        Process {
            pid: self.memory_map.len() as u64,  // Simple PID generation
            name: alloc::string::String::from(name),
            state: ProcessState::Ready,
        }
    }

    /// Load an ELF binary into memory (basic loader)
    pub fn load_elf(&mut self, data: &[u8]) -> Result<u64, &'static str> {
        // Validate ELF magic
        if data.len() < 64 || &data[0..4] != b"\x7fELF" {
            return Err("Invalid ELF magic");
        }

        // Check if it's 64-bit
        if data[4] != 2 {
            return Err("Not a 64-bit ELF");
        }

        // Parse header
        let entry = u64::from_le_bytes(data[24..32].try_into().unwrap());

        // TODO: Load program headers into memory
        // This is a minimal implementation that just returns the entry point
        // Full implementation would:
        // 1. Parse program headers
        // 2. Load PT_LOAD segments
        // 3. Handle relocations

        Ok(entry)
    }

    /// Execute an external application
    pub fn execute(&mut self, path: &str) -> Result<(), &'static str> {
        // For now, this is a stub that will be implemented
        // when we have proper filesystem access
        let _ = path;
        Err("Execution not yet implemented - needs filesystem and full ExitBootServices")
    }
}

/// Global runtime state (initialized after ExitBootServices)
static mut KERNEL_RUNTIME: Option<KernelRuntime> = None;

/// Initialize the kernel runtime (called after ExitBootServices)
pub unsafe fn init_runtime(memory_map: Vec<MemoryDescriptor>) {
    KERNEL_RUNTIME = Some(KernelRuntime::new(memory_map));
}

/// Get the global runtime instance
pub unsafe fn get_runtime() -> Option<&'static mut KernelRuntime> {
    KERNEL_RUNTIME.as_mut()
}

/// Check if we're in runtime mode
pub fn is_runtime_mode() -> bool {
    unsafe { KERNEL_RUNTIME.is_some() }
}
