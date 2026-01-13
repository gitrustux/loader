// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Kernel Runtime - Post-ExitBootServices
//!
//! This module handles the kernel runtime after exiting UEFI boot services.
//!
//! Runtime initialization order (MUST be followed):
//! 1. Memory allocator
//! 2. Exception handlers
//! 3. Interrupt controller
//! 4. Idle loop
//! 5. Scheduler stub

use alloc::vec::Vec;

/// Runtime initialization flags - track which components have been initialized
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInitFlags {
    pub memory_allocator: bool,
    pub exception_handlers: bool,
    pub interrupt_controller: bool,
    pub idle_loop: bool,
    pub scheduler: bool,
}

impl RuntimeInitFlags {
    pub const fn new() -> Self {
        Self {
            memory_allocator: false,
            exception_handlers: false,
            interrupt_controller: false,
            idle_loop: false,
            scheduler: false,
        }
    }

    pub fn is_fully_initialized(&self) -> bool {
        self.memory_allocator
            && self.exception_handlers
            && self.interrupt_controller
            && self.idle_loop
            && self.scheduler
    }
}

/// Simple bump allocator for kernel runtime
pub struct BumpAllocator {
    start: u64,
    end: u64,
    current: u64,
}

impl BumpAllocator {
    pub unsafe fn new(start: u64, size: u64) -> Self {
        Self {
            start,
            end: start + size,
            current: start,
        }
    }

    pub fn allocate(&mut self, size: u64, align: u64) -> Option<u64> {
        let aligned_current = (self.current + align - 1) & !(align - 1);
        let new_current = aligned_current + size;

        if new_current > self.end {
            return None; // Out of memory
        }

        self.current = new_current;
        Some(aligned_current)
    }

    pub fn available(&self) -> u64 {
        self.end - self.current
    }
}

/// Exception handler type
pub type ExceptionHandler = extern "C" fn(usize, usize, usize, usize);

/// Exception handler table (x86_64 has 32 exception vectors)
const NUM_EXCEPTION_VECTORS: usize = 32;

/// Exception handler table
static mut EXCEPTION_HANDLERS: [Option<ExceptionHandler>; NUM_EXCEPTION_VECTORS] = [None; NUM_EXCEPTION_VECTORS];

/// Default exception handler
extern "C" fn default_exception_handler(_error_code: usize, _rip: usize, _cs: usize, _rflags: usize) {
    // Heartbeat via serial port to show we hit an exception
    unsafe {
        // Send 'E' (Error) to COM1
        let com1 = 0x3F8u16 as *mut u8;
        com1.add(4).write_volatile(0); // Line control - enable DLAB
        com1.add(0).write_volatile(1); // Low byte of divisor (115200 baud)
        com1.add(1).write_volatile(0); // High byte of divisor
        com1.add(4).write_volatile(3); // 8N1
        com1.add(1).write_volatile(0); // Disable interrupts
        com1.write_volatile(b'E');     // Send 'E'
    }

    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

/// Install exception handler (IDT setup)
pub unsafe fn install_exception_handler(vector: usize, handler: ExceptionHandler) {
    if vector < NUM_EXCEPTION_VECTORS {
        EXCEPTION_HANDLERS[vector] = Some(handler);
    }
}

/// Initialize exception handlers (set up IDT)
unsafe fn init_exception_handlers_impl() -> bool {
    // Initialize all exception vectors with default handler
    for i in 0..NUM_EXCEPTION_VECTORS {
        EXCEPTION_HANDLERS[i] = Some(default_exception_handler);
    }

    // TODO: Set up actual IDT for x86_64
    // For now, just mark as initialized
    true
}

/// Interrupt controller state
pub struct InterruptController {
    pub enabled: bool,
}

impl InterruptController {
    pub const fn new() -> Self {
        Self { enabled: false }
    }

    pub fn enable(&mut self) {
        // TODO: Initialize APIC/PIC for x86_64
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        // TODO: Disable APIC/PIC
        self.enabled = false;
    }
}

/// Global interrupt controller
static mut INTERRUPT_CONTROLLER: Option<InterruptController> = None;

/// Initialize interrupt controller
unsafe fn init_interrupt_controller_impl() -> bool {
    let mut controller = InterruptController::new();
    controller.enable();
    INTERRUPT_CONTROLLER = Some(controller);
    true
}

/// Idle loop state
static mut IDLE_LOOP_RUNNING: bool = false;

/// Enter idle loop
pub fn idle_loop() -> ! {
    unsafe {
        IDLE_LOOP_RUNNING = true;

        loop {
            // Heartbeat via serial port - send '.' every second
            {
                let com1 = 0x3F8u16 as *mut u8;
                // Initialize COM1 once
                com1.add(4).write_volatile(0); // Line control - enable DLAB
                com1.add(0).write_volatile(1); // Low byte of divisor (115200 baud)
                com1.add(1).write_volatile(0); // High byte of divisor
                com1.add(4).write_volatile(3); // 8N1
                com1.add(1).write_volatile(0); // Disable interrupts

                // Send heartbeat
                com1.write_volatile(b'.');
            }

            // Check for scheduler work
            if let Some(runtime) = get_runtime() {
                if runtime.init_flags.scheduler {
                    // TODO: Run scheduler tick
                }
            }

            // Spin for approximately 1 second
            // On x86_64, assume ~3GHz, so ~3 billion cycles per second
            for _ in 0u64..3_000_000_000u64 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }
    }
}

/// Scheduler stub state
pub struct Scheduler {
    pub running: bool,
    pub current_task: Option<u64>,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            running: false,
            current_task: None,
        }
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn schedule(&mut self) {
        // TODO: Implement task scheduling
        // For now, this is a stub
    }
}

/// Global scheduler
static mut SCHEDULER: Option<Scheduler> = None;

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
    /// Runtime initialization flags
    pub init_flags: RuntimeInitFlags,
    /// Bump allocator for kernel runtime
    pub allocator: Option<*mut BumpAllocator>,
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
            init_flags: RuntimeInitFlags::new(),
            allocator: None,
        }
    }

    /// Initialize memory allocator
    pub unsafe fn init_allocator(&mut self) -> Result<(), &'static str> {
        // Find a large block of conventional memory for the allocator
        // Use 256 pages (1MB) for the kernel heap
        let heap_start = self.find_free_memory(256)
            .ok_or("No free memory for allocator")?;

        let heap_size = 256 * 4096; // 1MB

        // Create the allocator
        let allocator = BumpAllocator::new(heap_start, heap_size);

        // Store it in a leaked box to get a stable pointer
        let allocator_box = alloc::boxed::Box::leak(alloc::boxed::Box::new(allocator));

        self.allocator = Some(allocator_box);

        // Send 'A' to serial to indicate allocator initialized
        let com1 = 0x3F8u16 as *mut u8;
        com1.add(4).write_volatile(0);
        com1.add(0).write_volatile(1);
        com1.add(1).write_volatile(0);
        com1.add(4).write_volatile(3);
        com1.add(1).write_volatile(0);
        com1.write_volatile(b'A');

        self.init_flags.memory_allocator = true;
        Ok(())
    }

    /// Initialize exception handlers
    pub unsafe fn init_exception_handlers(&mut self) -> Result<(), &'static str> {
        if !init_exception_handlers_impl() {
            return Err("Failed to initialize exception handlers");
        }

        // Send 'X' to serial to indicate exception handlers initialized
        let com1 = 0x3F8u16 as *mut u8;
        com1.add(4).write_volatile(0);
        com1.add(0).write_volatile(1);
        com1.add(1).write_volatile(0);
        com1.add(4).write_volatile(3);
        com1.add(1).write_volatile(0);
        com1.write_volatile(b'X');

        self.init_flags.exception_handlers = true;
        Ok(())
    }

    /// Initialize interrupt controller
    pub unsafe fn init_interrupt_controller(&mut self) -> Result<(), &'static str> {
        if !init_interrupt_controller_impl() {
            return Err("Failed to initialize interrupt controller");
        }

        // Send 'I' to serial to indicate interrupt controller initialized
        let com1 = 0x3F8u16 as *mut u8;
        com1.add(4).write_volatile(0);
        com1.add(0).write_volatile(1);
        com1.add(1).write_volatile(0);
        com1.add(4).write_volatile(3);
        com1.add(1).write_volatile(0);
        com1.write_volatile(b'I');

        self.init_flags.interrupt_controller = true;
        Ok(())
    }

    /// Initialize idle loop
    pub fn init_idle_loop(&mut self) {
        self.init_flags.idle_loop = true;
    }

    /// Initialize scheduler
    pub unsafe fn init_scheduler(&mut self) -> Result<(), &'static str> {
        let mut scheduler = Scheduler::new();
        scheduler.start();

        SCHEDULER = Some(scheduler);

        // Send 'S' to serial to indicate scheduler initialized
        let com1 = 0x3F8u16 as *mut u8;
        com1.add(4).write_volatile(0);
        com1.add(0).write_volatile(1);
        com1.add(1).write_volatile(0);
        com1.add(4).write_volatile(3);
        com1.add(1).write_volatile(0);
        com1.write_volatile(b'S');

        self.init_flags.scheduler = true;
        Ok(())
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
