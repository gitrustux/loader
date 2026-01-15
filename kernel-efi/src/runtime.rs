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

/// x86_64 IDT Entry (Interrupt Descriptor Table Entry)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct IdtEntry {
    /// Lower 16 bits of handler address
    offset_low: u16,
    /// Kernel code segment selector
    selector: u16,
    /// Interrupt Stack Table index (0 for most cases)
    ist: u8,
    /// Type and attributes
    type_attr: u8,
    /// Middle 16 bits of handler address
    offset_mid: u16,
    /// Upper 32 bits of handler address
    offset_high: u32,
    /// Reserved (must be 0)
    reserved: u32,
}

impl IdtEntry {
    /// Create a new IDT entry
    pub const fn new(handler: u64, selector: u16, type_attr: u8) -> Self {
        Self {
            offset_low: (handler & 0xFFFF) as u16,
            selector,
            ist: 0,
            type_attr,
            offset_mid: ((handler >> 16) & 0xFFFF) as u16,
            offset_high: ((handler >> 32) & 0xFFFFFFFF) as u32,
            reserved: 0,
        }
    }

    /// Create an interrupt gate entry (for x86_64 exceptions/interrupts)
    const fn interrupt_gate(handler: u64, selector: u16) -> Self {
        // Type attributes for 64-bit interrupt gate:
        // - Present: 0x80 (bit 7)
        // - DPL: 0x00 (bits 5-6, kernel level)
        // - Storage: 0 (bit 4)
        // - Type: 0x0E (bits 0-3, interrupt gate)
        Self::new(handler, selector, 0x8E)
    }

    /// Create a trap gate entry (for some exceptions)
    const fn trap_gate(handler: u64, selector: u16) -> Self {
        // Type attributes for 64-bit trap gate:
        // - Present: 0x80 (bit 7)
        // - DPL: 0x00 (bits 5-6, kernel level)
        // - Storage: 0 (bit 4)
        // - Type: 0x0F (bits 0-3, trap gate)
        Self::new(handler, selector, 0x8F)
    }

    /// Create an absent entry
    const fn absent() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

/// x86_64 IDT Pointer structure for lidt instruction
#[repr(C, packed)]
pub struct IdtPointer {
    /// Size of IDT - 1
    limit: u16,
    /// Base address of IDT
    base: u64,
}

impl IdtPointer {
    fn new(base: u64, limit: u16) -> Self {
        Self { limit, base }
    }
}

/// Interrupt Descriptor Table (256 entries for x86_64)
/// We only use the first 32 for CPU exceptions, the rest are for IRQs
pub static mut IDT: [IdtEntry; 256] = [IdtEntry::absent(); 256];

/// Exception frame pushed by x86_64 CPU
#[repr(C)]
pub struct ExceptionFrame {
    /// Instruction pointer (RIP)
    pub rip: u64,
    /// Code segment (CS)
    pub cs: u64,
    /// RFLAGS register
    pub rflags: u64,
    /// Stack pointer (RSP)
    pub rsp: u64,
    /// Stack segment (SS)
    pub ss: u64,
}

/// Exception handler wrapper - called from assembly stubs
extern "C" fn exception_handler_wrapper(vector: usize, error_code: u64, frame: &ExceptionFrame) {
    // Get the handler for this exception
    let handler = unsafe {
        EXCEPTION_HANDLERS.get(vector)
            .and_then(|h| *h)
            .unwrap_or(default_exception_handler)
    };

    // Call the handler with error code, RIP, CS, and RFLAGS
    handler(
        error_code as usize,
        frame.rip as usize,
        frame.cs as usize,
        frame.rflags as usize,
    );
}

/// Macro to generate exception handler stubs
macro_rules! define_exception_stub {
    ($name:ident, $vector:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() -> ! {
            // For exceptions without error code (most of them)
            // Stack layout when handler is called:
            // [old RSP] [SS] [old RSP] [RFLAGS] [CS] [RIP] [error code (optional)]
            // We need to push a dummy error code for exceptions that don't have one

            core::arch::naked_asm!(
                // Push dummy error code (0) for consistency
                "push 0",
                // Push exception vector
                "push {vector}",
                // Save all general-purpose registers
                "push rax",
                "push rbx",
                "push rcx",
                "push rdx",
                "push rsi",
                "push rdi",
                "push rbp",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                // Save data segment
                "mov rdi, ds",
                "push rdi",
                // Load kernel data segment
                "mov ax, 0x10",  // Assuming kernel data selector is 0x10
                "mov ds, ax",
                "mov es, ax",
                "mov fs, ax",
                "mov gs, ax",
                // Call the wrapper: exception_handler_wrapper(vector, error_code, &frame)
                // Stack already has: [error_code] [vector] [saved regs...]
                // We need to pass: RDI=vector, RSI=error_code, RDX=&frame
                "mov rdi, rsp",  // RDI = pointer to saved regs (contains error_code at offset)
                "add rdi, 8",     // Skip past dummy error code to get vector
                "mov rsi, [rsp-8]",  // RSI = error_code (from dummy push)
                "mov rdx, rsp",  // RDX = stack pointer (as ExceptionFrame*)
                "add rdx, 9*8",  // Skip past saved regs to get to exception frame
                "call {wrapper}",
                // Restore data segment
                "pop rdi",
                "mov ds, rdi",
                "mov es, rdi",
                "mov fs, rdi",
                "mov gs, rdi",
                // Restore registers
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rbp",
                "pop rdi",
                "pop rsi",
                "pop rdx",
                "pop rcx",
                "pop rbx",
                "pop rax",
                // Clean up error code and vector
                "add rsp, 16",
                // Return from exception
                "iretq",
                vector = const $vector,
                wrapper = sym exception_handler_wrapper
            );
        }
    };
}

// Define exception stubs for all 32 exception vectors
define_exception_stub!(exception_stub_0, 0);   // Division by zero
define_exception_stub!(exception_stub_1, 1);   // Debug
define_exception_stub!(exception_stub_2, 2);   // Non-maskable interrupt
define_exception_stub!(exception_stub_3, 3);   // Breakpoint
define_exception_stub!(exception_stub_4, 4);   // Overflow
define_exception_stub!(exception_stub_5, 5);   // Bound range exceeded
define_exception_stub!(exception_stub_6, 6);   // Invalid opcode
define_exception_stub!(exception_stub_7, 7);   // Device not available
define_exception_stub!(exception_stub_8, 8);   // Double fault (has error code)
define_exception_stub!(exception_stub_9, 9);   // Coprocessor segment overrun
define_exception_stub!(exception_stub_10, 10); // Invalid TSS
define_exception_stub!(exception_stub_11, 11); // Segment not present
define_exception_stub!(exception_stub_12, 12); // Stack segment fault
define_exception_stub!(exception_stub_13, 13); // General protection fault
define_exception_stub!(exception_stub_14, 14); // Page fault
define_exception_stub!(exception_stub_15, 15); // Reserved
define_exception_stub!(exception_stub_16, 16); // x87 FPU error
define_exception_stub!(exception_stub_17, 17); // Alignment check
define_exception_stub!(exception_stub_18, 18); // Machine check
define_exception_stub!(exception_stub_19, 19); // SIMD floating-point exception
define_exception_stub!(exception_stub_20, 20); // Virtualization exception
define_exception_stub!(exception_stub_21, 21); // Reserved
define_exception_stub!(exception_stub_22, 22); // Reserved
define_exception_stub!(exception_stub_23, 23); // Reserved
define_exception_stub!(exception_stub_24, 24); // Reserved
define_exception_stub!(exception_stub_25, 25); // Reserved
define_exception_stub!(exception_stub_26, 26); // Reserved
define_exception_stub!(exception_stub_27, 27); // Reserved
define_exception_stub!(exception_stub_28, 28); // Reserved
define_exception_stub!(exception_stub_29, 29); // Reserved
define_exception_stub!(exception_stub_30, 30); // Reserved
define_exception_stub!(exception_stub_31, 31); // Reserved

/// Array of exception stub pointers
static EXCEPTION_STUBS: [unsafe extern "C" fn() -> !; 32] = [
    exception_stub_0, exception_stub_1, exception_stub_2, exception_stub_3,
    exception_stub_4, exception_stub_5, exception_stub_6, exception_stub_7,
    exception_stub_8, exception_stub_9, exception_stub_10, exception_stub_11,
    exception_stub_12, exception_stub_13, exception_stub_14, exception_stub_15,
    exception_stub_16, exception_stub_17, exception_stub_18, exception_stub_19,
    exception_stub_20, exception_stub_21, exception_stub_22, exception_stub_23,
    exception_stub_24, exception_stub_25, exception_stub_26, exception_stub_27,
    exception_stub_28, exception_stub_29, exception_stub_30, exception_stub_31,
];

/// ============================================================================
/// PHASE 9: Keyboard Interrupt Handler (IRQ1)
/// ============================================================================
///
/// IRQ1 is the keyboard interrupt on x86_64.
/// It corresponds to IDT vector 33 (32 + 1).
///

/// Keyboard IRQ1 interrupt handler stub
#[unsafe(naked)]
unsafe extern "C" fn keyboard_irq_stub() -> ! {
    core::arch::naked_asm!(
        // Save all general-purpose registers
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        // Call the keyboard handler
        "call {handler}",
        // Restore registers
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        // Send EOI to PIC
        "mov al, 0x20",
        "out 0x20, al",
        // Return from interrupt
        "iretq",
        handler = sym crate::keyboard::keyboard_irq_handler
    );
}

/// PIC (Programmable Interrupt Controller) I/O ports
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// Initialize the PIC for IRQ handling
///
/// This function remaps the PIC IRQs to vectors 32-47 and enables
/// the keyboard IRQ (IRQ1).
unsafe fn init_pic() {
    // ICW1: Initialize PIC, requires ICW4
    outb(PIC1_COMMAND, 0x11);
    outb(PIC2_COMMAND, 0x11);

    // ICW2: Vector offset (32 for PIC1, 40 for PIC2)
    outb(PIC1_DATA, 0x20); // PIC1: IRQ0-7 -> vectors 32-39
    outb(PIC2_DATA, 0x28); // PIC2: IRQ8-15 -> vectors 40-47

    // ICW3: PIC wiring (PIC2 is at IRQ2 on PIC1)
    outb(PIC1_DATA, 0x04);
    outb(PIC2_DATA, 0x02);

    // ICW4: 8086 mode
    outb(PIC1_DATA, 0x01);
    outb(PIC2_DATA, 0x01);

    // Enable IRQ1 (keyboard) on PIC1
    // IRQ mask: 0 = enabled, 1 = disabled
    // We enable ONLY IRQ1 (keyboard) for now
    // IRQ0 (timer) is DISABLED to allow HLT to actually idle the CPU
    outb(PIC1_DATA, 0xFD); // 0xFD = 11111101 (IRQ1 enabled, IRQ0 disabled)
    outb(PIC2_DATA, 0xFF); // All IRQs on PIC2 disabled
}

/// Output byte to I/O port
#[inline(always)]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack)
    );
}

/// Initialize keyboard interrupt handler
pub unsafe fn init_keyboard_interrupts() -> Result<(), &'static str> {
    // Initialize PIC
    init_pic();

    // Get kernel code segment selector
    let kernel_cs: u16;
    core::arch::asm!(
        "mov {0:x}, cs",
        out(reg) kernel_cs,
        options(nomem, nostack, preserves_flags)
    );

    // Set up IDT entry for IRQ1 (keyboard) at vector 33
    let keyboard_handler = keyboard_irq_stub as u64;
    IDT[33] = IdtEntry::interrupt_gate(keyboard_handler, kernel_cs);

    // Initialize keyboard driver
    crate::keyboard::init();

    Ok(())
}

/// Default exception handler
extern "C" fn default_exception_handler(_error_code: usize, rip: usize, _cs: usize, _rflags: usize) {
    // Write exception info to VGA
    unsafe {
        let vga_buffer = 0xB8000u64 as *mut u16;

        // Clear top line and write exception message
        let msg = b"EXCEPTION! RIP=";
        let mut ptr = vga_buffer;
        for (i, &byte) in msg.iter().enumerate() {
            if i < 80 {
                *ptr.add(i) = 0x4F00 | (byte as u16); // Red on white
            }
        }

        // Write RIP in hex at end of message
        let mut ptr = vga_buffer.add(msg.len());
        let hex = b"0123456789ABCDEF";
        let mut rip_val = rip as u64;
        for i in 0..16 {
            let nibble = (rip_val >> (60 - i * 4)) & 0xF;
            *ptr.add(i) = 0x4F00 | (hex[nibble as usize] as u16);
        }
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

/// Initialize exception handlers (set up IDT) - public wrapper
pub unsafe fn init_exception_handlers() -> Result<(), &'static str> {
    if init_exception_handlers_impl() {
        Ok(())
    } else {
        Err("Failed to initialize exception handlers")
    }
}

/// Load the IDT using the lidt instruction
#[inline(always)]
unsafe fn load_idt(idt_ptr: &IdtPointer) {
    core::arch::asm!(
        "lidt [{}]",
        in(reg) idt_ptr,
        options(nostack, preserves_flags, readonly)
    );
}

/// Initialize exception handlers (set up IDT)
unsafe fn init_exception_handlers_impl() -> bool {
    // Initialize all exception vectors with default handler
    for i in 0..NUM_EXCEPTION_VECTORS {
        EXCEPTION_HANDLERS[i] = Some(default_exception_handler);
    }

    // Get kernel code segment selector
    // In UEFI/x86_64, the code selector is typically the current CS
    // We'll read it using a special instruction or assume a standard value
    let kernel_cs: u16;
    core::arch::asm!(
        "mov {0:x}, cs",
        out(reg) kernel_cs,
        options(nomem, nostack, preserves_flags)
    );

    // Set up IDT entries for exceptions (vectors 0-31)
    // Use interrupt gates for most, trap gate for breakpoint (3)
    for i in 0..32 {
        let handler = EXCEPTION_STUBS[i] as u64;
        IDT[i] = if i == 3 {
            // Breakpoint exception uses trap gate (doesn't disable interrupts)
            IdtEntry::trap_gate(handler, kernel_cs)
        } else {
            // All others use interrupt gate
            IdtEntry::interrupt_gate(handler, kernel_cs)
        };
    }

    // Create IDT pointer and load it
    let idt_ptr = IdtPointer::new(
        &IDT as *const _ as u64,
        (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16
    );

    load_idt(&idt_ptr);

    // Write success message to VGA
    let vga_buffer = 0xB8000u64 as *mut u16;
    let msg = "IDT OK!";
    let mut ptr = vga_buffer.add(70); // Column 70
    for (i, &byte) in msg.as_bytes().iter().enumerate() {
        if i < 10 {
            *ptr.add(i) = 0x0E00 | (byte as u16); // Yellow on black
        }
    }

    true
}

/// Local APIC MMIO register offsets
#[repr(C)]
pub struct LocalApicRegisters {
    _reserved0: [u32; 2],           // 0x00-0x07
    id: u32,                        // 0x08 - Local APIC ID
    _reserved1: [u32; 3],           // 0x0C-0x17
    version: u32,                   // 0x1C - Local APIC Version
    _reserved2: [u32; 4],           // 0x20-0x2F
    tpr: u32,                       // 0x30 - Task Priority Register
    _reserved3: [u32; 3],           // 0x34-0x3F
    eoi: u32,                       // 0x40 - EOI Register
    _reserved4: [u32; 3],           // 0x44-0x4F
    ldr: u32,                       // 0x50 - Logical Destination Register
    _reserved5: [u32; 3],           // 0x54-0x5F
    dfr: u32,                       // 0x60 - Destination Format Register
    _reserved6: [u32; 3],           // 0x64-0x6F
    svr: u32,                       // 0x70 - Spurious Interrupt Vector Register
    _reserved7: [u32; 3],           // 0x74-0x7F
    isr0: u32,                      // 0x80 - In-Service Register 0
    isr1: u32,                      // 0x84 - In-Service Register 1
    isr2: u32,                      // 0x88 - In-Service Register 2
    isr3: u32,                      // 0x8C - In-Service Register 3
    isr4: u32,                      // 0x90 - In-Service Register 4
    isr5: u32,                      // 0x94 - In-Service Register 5
    isr6: u32,                      // 0x98 - In-Service Register 6
    isr7: u32,                      // 0x9C - In-Service Register 7
    tmr0: u32,                      // 0xA0 - Trigger Mode Register 0
    tmr1: u32,                      // 0xA4 - Trigger Mode Register 1
    tmr2: u32,                      // 0xA8 - Trigger Mode Register 2
    tmr3: u32,                      // 0xAC - Trigger Mode Register 3
    tmr4: u32,                      // 0xB0 - Trigger Mode Register 4
    tmr5: u32,                      // 0xB4 - Trigger Mode Register 5
    tmr6: u32,                      // 0xB8 - Trigger Mode Register 6
    tmr7: u32,                      // 0xBC - Trigger Mode Register 7
    irr0: u32,                      // 0xC0 - Interrupt Request Register 0
    irr1: u32,                      // 0xC4 - Interrupt Request Register 1
    irr2: u32,                      // 0xC8 - Interrupt Request Register 2
    irr3: u32,                      // 0xCC - Interrupt Request Register 3
    irr4: u32,                      // 0xD0 - Interrupt Request Register 4
    irr5: u32,                      // 0xD4 - Interrupt Request Register 5
    irr6: u32,                      // 0xD8 - Interrupt Request Register 6
    irr7: u32,                      // 0xDC - Interrupt Request Register 7
    error_status: u32,              // 0xE0 - Error Status Register
    _reserved8: [u32; 5],           // 0xE4-0xF7
    icr_low: u32,                   // 0xF0 - Interrupt Command Register Low
    icr_high: u32,                  // 0xF4 - Interrupt Command Register High
    _reserved9: [u32; 2],           // 0xF8-0xFF
    timer_lvt: u32,                 // 0x100 - Timer Local Vector Table
    thermal_lvt: u32,               // 0x110 - Thermal Monitor LVT
    perf_lvt: u32,                  // 0x120 - Performance Counter LVT
    lint0: u32,                     // 0x130 - Local Interrupt 0 (LINT0)
    lint1: u32,                     // 0x140 - Local Interrupt 1 (LINT1)
    error_lvt: u32,                 // 0x150 - Error LVT
    _reserved10: [u32; 3],          // 0x160-0x16F
    timer_initial: u32,             // 0x170 - Timer Initial Count
    timer_current: u32,             // 0x180 - Timer Current Count
    _reserved11: [u32; 2],          // 0x190-0x19F
    timer_divide: u32,              // 0x1A0 - Timer Divide Configuration
    _reserved12: [u32; 1],          // 0x1A4-0x1A7
}

impl LocalApicRegisters {
    /// Write to a register with volatile semantics
    #[inline]
    unsafe fn write_reg(&self, offset: usize, value: u32) {
        let base = self as *const _ as usize;
        let ptr = (base + offset) as *mut u32;
        ptr.write_volatile(value);
    }

    /// Read from a register with volatile semantics
    #[inline]
    unsafe fn read_reg(&self, offset: usize) -> u32 {
        let base = self as *const _ as usize;
        let ptr = (base + offset) as *const u32;
        ptr.read_volatile()
    }
}

/// Local APIC base address (default from x86_64 CPU)
const LOCAL_APIC_DEFAULT_BASE: u64 = 0xFEE0_0000;

/// Local APIC mapped address (will be set during initialization)
static mut LOCAL_APIC_ADDRESS: Option<&'static mut LocalApicRegisters> = None;

/// Interrupt controller state
pub struct InterruptController {
    pub enabled: bool,
    apic_base: u64,
}

impl InterruptController {
    pub const fn new() -> Self {
        Self {
            enabled: false,
            apic_base: LOCAL_APIC_DEFAULT_BASE,
        }
    }

    /// Initialize the Local APIC
    pub unsafe fn enable(&mut self) -> bool {
        // Map Local APIC MMIO region
        let apic_addr = self.apic_base as *mut LocalApicRegisters;
        LOCAL_APIC_ADDRESS = Some(&mut *apic_addr);

        if let Some(ref apic) = LOCAL_APIC_ADDRESS {
            // Enable APIC via Spurious Interrupt Vector Register (offset 0x70)
            // Bit 8: APIC Software Enable/Disable
            // Bits 0-7: Spurious Vector
            apic.write_reg(0x70, 0x100 | 0xFF); // Enable APIC, set spurious vector to 0xFF

            // Set Task Priority Register to 0 (allow all interrupts) (offset 0x30)
            apic.write_reg(0x30, 0);

            // Disable all LVT entries initially
            apic.write_reg(0x100, 0x10000); // Mask timer (offset 0x100)
            apic.write_reg(0x110, 0x10000); // Mask thermal (offset 0x110)
            apic.write_reg(0x120, 0x10000); // Mask perf (offset 0x120)
            apic.write_reg(0x130, 0x10000); // Mask LINT0 (offset 0x130)
            apic.write_reg(0x140, 0x10000); // Mask LINT1 (offset 0x140)
            apic.write_reg(0x150, 0x10000); // Mask error (offset 0x150)

            self.enabled = true;
            return true;
        }

        false
    }

    pub fn disable(&mut self) {
        // Disable APIC
        unsafe {
            if let Some(ref apic) = LOCAL_APIC_ADDRESS {
                // Disable APIC via SVR (offset 0x70, clear bit 8)
                apic.write_reg(0x70, 0xFF);
            }
        }
        self.enabled = false;
    }

    /// Send End of Interrupt (EOI) to the APIC
    pub unsafe fn send_eoi(&self) {
        if let Some(ref apic) = LOCAL_APIC_ADDRESS {
            // EOI register is at offset 0x40
            apic.write_reg(0x40, 0);
        }
    }
}

/// Global interrupt controller
static mut INTERRUPT_CONTROLLER: Option<InterruptController> = None;

/// Global scheduler
static mut SCHEDULER: Option<Scheduler> = None;

/// Initialize interrupt controller
unsafe fn init_interrupt_controller_impl() -> bool {
    let mut controller = InterruptController::new();
    if controller.enable() {
        INTERRUPT_CONTROLLER = Some(controller);
        true
    } else {
        false
    }
}

/// Public wrapper for interrupt controller initialization
pub unsafe fn init_interrupt_controller() -> Result<(), &'static str> {
    if init_interrupt_controller_impl() {
        Ok(())
    } else {
        Err("Failed to initialize interrupt controller")
    }
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

/// Initialize scheduler stub (Phase 5)
pub unsafe fn init_scheduler_stub() -> Result<(), &'static str> {
    let mut scheduler = Scheduler::new();
    scheduler.start();
    SCHEDULER = Some(scheduler);
    Ok(())
}

/// Global allocator for kernel runtime (static storage, no allocation needed)
static mut KERNEL_ALLOCATOR_INITIALIZED: bool = false;
static mut KERNEL_ALLOCATOR_STORAGE: BumpAllocator = BumpAllocator {
    start: 0,
    end: 0,
    current: 0,
};

/// Initialize the kernel allocator from memory map (must be called first!)
/// This function does NOT use the global allocator - it sets it up.
///
/// # Safety
/// Must be called exactly once after ExitBootServices.
pub unsafe fn init_kernel_allocator_from_memory_map(
    memory_map_buffer: *const u8,
    map_size: usize,
    entry_size: usize,
) -> Result<(), &'static str> {
    use uefi_raw::table::boot::MemoryDescriptor;
    use uefi_raw::table::boot::MemoryType;

    // Find the largest conventional memory region
    let desc_ptr = memory_map_buffer as *const MemoryDescriptor;
    let mut offset = 0;
    let mut best_start: u64 = 0;
    let mut best_size: u64 = 0;

    while offset < map_size {
        let desc = &*desc_ptr.add(offset / entry_size);

        // Check memory type - look for conventional memory or reusable boot memory
        // Use matches! macro to properly compare MemoryType enum
        let is_usable = matches!(desc.ty,
            MemoryType::CONVENTIONAL      // Type 7: EfiConventionalMemory
            | MemoryType::LOADER_CODE     // Type 1: EfiLoaderCode
            | MemoryType::LOADER_DATA     // Type 2: EfiLoaderData
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA
        );

        if is_usable && desc.page_count > 256 {
            // Found a suitable region
            if desc.page_count > best_size / 4096 {
                best_start = desc.phys_start;
                best_size = desc.page_count * 4096;
            }
        }

        offset += entry_size;
    }

    if best_size < 1024 * 1024 {
        return Err("No suitable memory region found (need at least 1MB)");
    }

    // Initialize the bump allocator with a portion of this memory
    // Reserve first 1MB for the allocator heap
    let heap_start = best_start;
    let heap_size = 1024 * 1024; // 1MB

    KERNEL_ALLOCATOR_STORAGE = BumpAllocator::new(heap_start, heap_size);
    KERNEL_ALLOCATOR_INITIALIZED = true; // Mark as initialized

    // Write success message to VGA
    let vga_buffer = 0xB8000u64 as *mut u16;
    let msg = "ALLOC OK!";
    let mut ptr = vga_buffer.add(40); // Start at column 40
    for (i, &byte) in msg.as_bytes().iter().enumerate() {
        if i < 10 {
            *ptr.add(i) = 0x0E00 | (byte as u16); // Yellow on black
        }
    }

    Ok(())
}

/// Get the kernel allocator (returns None if not initialized)
pub unsafe fn get_kernel_allocator() -> Option<&'static mut BumpAllocator> {
    if KERNEL_ALLOCATOR_INITIALIZED {
        Some(&mut KERNEL_ALLOCATOR_STORAGE)
    } else {
        None
    }
}

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

/// ========================================================================
/// PHASE 6: Disable UEFI Services Permanently
/// ========================================================================
///
/// After ExitBootServices, UEFI boot services are no longer available.
/// This module provides functions to permanently disable UEFI services
/// and prevent accidental use of UEFI APIs in runtime mode.
///

use uefi_raw::table::system::SystemTable;

/// Flag indicating if UEFI services have been permanently disabled
static mut UEFI_SERVICES_DISABLED: bool = false;

/// UEFI system table pointer (set to None after disabling)
static mut UEFI_SYSTEM_TABLE: Option<*const SystemTable> = None;

/// Set the UEFI system table pointer (called during kernel init)
pub unsafe fn set_uefi_system_table(table: *const SystemTable) {
    UEFI_SYSTEM_TABLE = Some(table);
}

/// Disable UEFI services permanently
///
/// This function:
/// 1. Zeros out the UEFI system table pointer
/// 2. Marks UEFI services as disabled
/// 3. Writes confirmation to VGA
///
/// # Safety
/// Must be called AFTER ExitBootServices returns successfully.
/// Calling this before ExitBootServices will cause undefined behavior.
pub unsafe fn disable_uefi_services_permanently() -> Result<(), &'static str> {
    // Already disabled?
    if UEFI_SERVICES_DISABLED {
        return Ok(());
    }

    // Clear the system table pointer to prevent accidental use
    UEFI_SYSTEM_TABLE = None;

    // Mark as disabled
    UEFI_SERVICES_DISABLED = true;

    // Write confirmation to VGA
    const VGA_BUFFER: u64 = 0xB8000;
    let vga_buffer = VGA_BUFFER as *mut u16;
    let msg = "UEFI DISABLED!";
    let mut ptr = vga_buffer.add(130); // Column 130 (after KBD status)
    for (i, &byte) in msg.as_bytes().iter().enumerate() {
        if i < 15 {
            *ptr.add(i) = 0x0900 | (byte as u16); // Blue on black
        }
    }

    Ok(())
}

/// Check if UEFI services are disabled
pub fn is_uefi_disabled() -> bool {
    unsafe { UEFI_SERVICES_DISABLED }
}

/// Panic if UEFI services are still accessible
///
/// This is a safety function that can be called at the beginning
/// of any function that should only run in runtime mode.
#[inline(always)]
pub fn assert_runtime_mode() {
    if !is_uefi_disabled() {
        // UEFI services still accessible - this is a bug!
        unsafe {
            const VGA_BUFFER: u64 = 0xB8000;
            let vga_buffer = VGA_BUFFER as *mut u16;
            let msg = b"ERROR: UEFI NOT DISABLED!";
            for (i, &byte) in msg.iter().enumerate() {
                if i < 80 {
                    *vga_buffer.add(i) = 0x4F00 | (byte as u16); // Red on white
                }
            }
        }
        loop {
            unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
        }
    }
}

/// Get UEFI system table pointer (returns None if disabled)
///
/// This function prevents accidental use of UEFI services after
/// they have been disabled.
pub unsafe fn get_uefi_system_table() -> Option<*const SystemTable> {
    if UEFI_SERVICES_DISABLED {
        None
    } else {
        UEFI_SYSTEM_TABLE
    }
}
