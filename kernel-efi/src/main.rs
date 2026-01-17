#![no_std]
#![no_main]

extern crate alloc;

use core::panic::PanicInfo;
use uefi::prelude::*;
use uefi::boot;
use uefi::mem::memory_map::MemoryMap;

// Use UEFI allocator as global allocator
#[global_allocator]
static ALLOCATOR: uefi::allocator::Allocator = uefi::allocator::Allocator;

mod console;
mod framebuffer;
mod runtime;
mod theme;
mod keyboard;
mod shell;
mod syscall;
mod userspace_bin;
mod filesystem;
mod vga_console;
mod native_console;

/// Exit UEFI boot services with frozen zone enforcement
///
/// This function implements the "frozen zone" pattern:
/// 1. Memory barrier to prevent reordering before ExitBootServices
/// 2. NO UEFI calls, allocations, or logging during the frozen zone
/// 3. Memory barrier to ensure ExitBootServices completes
/// 4. The uefi 0.36 crate internally handles:
///    - Memory map capture
///    - Retry logic if memory map changes
///    - Up to MAX_EXIT_BOOT_SERVICES_RETRIES attempts
///
/// Returns the owned memory map after successful exit (as an opaque type).
///
/// Note: The uefi 0.36 `exit_boot_services` function already implements
/// the retry loop and will panic if it cannot exit after many attempts.
fn exit_boot_services_with_retry() -> impl MemoryMap {
    // =========================================================
    // FROZEN ZONE BEGINS
    // =========================================================
    // ABSOLUTELY NO UEFI CALLS, ALLOCATIONS, OR LOGGING BELOW
    // (until ExitBootServices completes)

    // Memory barrier: prevent compiler/CPU from reordering operations
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    // The uefi 0.36 crate handles:
    // - Memory map capture
    // - Retry logic (memory map may change)
    // - ExitBootServices call
    let memory_map = unsafe { boot::exit_boot_services(None) };

    // Memory barrier: ensure ExitBootServices is complete
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    // =========================================================
    // FROZEN ZONE ENDS
    // =========================================================

    memory_map
}

/// Panic handler
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    console::emergency_write("KERNEL PANIC - HALTING");
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

/// Entry point
#[entry]
fn main() -> Status {
    // =========================================================
    // PHASE 1: BOOT SERVICES MODE
    // =========================================================

    // Init boot console (UEFI stdout ONLY)
    console::init_boot_console();
    console::write_line("Rustux kernel starting (boot services)");

    // Initialize GOP + framebuffer metadata
    console::write_line("Querying GOP framebuffer...");
    framebuffer::init();
    console::write_line("Framebuffer available");

    // ============================================================
    // CRITICAL: Verify framebuffer initialization before ExitBootServices
    // ============================================================
    if !framebuffer::is_initialized() {
        console::write_line("ERROR: Framebuffer not initialized!");
        loop {
            unsafe { core::arch::asm!("hlt") };
        }
    }

    // Capture memory map NOW (do not touch UEFI after this)
    console::write_line("Preparing to exit boot services...");

    // Switch console target BEFORE exit
    console::switch_to_framebuffer_console();

    // ============================================================
    // ==================== FROZEN ZONE ====================
    // ============================================================
    // CRITICAL: From this point until ExitBootServices completes:
    //
    // - NO allocations
    // - NO logging
    // - NO UEFI calls
    // - ONLY ExitBootServices
    //
    // Any violation will cause the firmware to reclaim memory
    // or corrupt the memory map, leading to undefined behavior.
    // ============================================================

    // POST CODE 0x10: About to call ExitBootServices
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x80u16,
            in("al") 0x10u8,
            options(nomem, nostack)
        );
    }

    // Exit boot services with frozen zone enforcement
    // This returns the memory map that we'll use for allocator initialization
    let memory_map = exit_boot_services_with_retry();

    // POST CODE 0x20: ExitBootServices succeeded
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x80u16,
            in("al") 0x20u8,
            options(nomem, nostack)
        );
    }

    // =========================================================
    // PHASE 3: RUNTIME MODE
    // =========================================================

    // UEFI IS DEAD FROM HERE ON
    // ----------------------------------------

    // Disable interrupts
    unsafe { core::arch::asm!("cli") };

    framebuffer::clear();
    framebuffer::write_str("ExitBootServices OK\n");

    // POST CODE 0x30: In runtime mode
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x80u16,
            in("al") 0x30u8,
            options(nomem, nostack)
        );
    }

    // =========================================================
    // PHASE 4: ALLOCATOR INITIALIZATION
    // =========================================================

    framebuffer::write_str("Initializing kernel allocator...\n");

    // Initialize the kernel allocator from the UEFI memory map
    // The uefi 0.36 MemoryMap provides access to memory descriptors
    // We need to pass the raw memory map data to the allocator init function

    // Find the largest conventional memory region for the allocator
    let mut best_region: Option<(u64, u64)> = None; // (physical_start, size)
    let mut best_size: u64 = 0;

    for entry in memory_map.entries() {
        // Look for conventional memory (type 7 = EfiConventionalMemory)
        // We use the raw type value to avoid import issues
        let ty_value: u32 = unsafe { core::mem::transmute(entry.ty) };
        if ty_value == 7 {
            let start = entry.phys_start;
            let pages = entry.page_count;
            let size = pages * 4096;

            if size > best_size {
                best_size = size;
                best_region = Some((start, size));
            }
        }
    }

    match best_region {
        Some((start, size)) if size >= 1024 * 1024 => {
            // Found at least 1MB of conventional memory
            // Initialize the kernel allocator with this region
            unsafe {
                // Initialize bump allocator with 1MB heap from the found region
                let heap_start = start;
                let heap_size = 1024 * 1024; // 1MB

                // Call the allocator initialization directly
                // This is a simplified version that doesn't need the full memory map
                if let Err(e) = runtime::init_kernel_allocator_simple(heap_start, heap_size) {
                    framebuffer::write_str("Allocator init FAILED: ");
                    framebuffer::write_str(e);
                    framebuffer::write_str("\nHALTING\n");
                    loop {
                        core::arch::asm!("hlt");
                    }
                }

                framebuffer::write_str("Allocator initialized OK\n");
            }
        }
        Some((_, _size)) => {
            // Not enough memory
            framebuffer::write_str("ERROR: Not enough conventional memory (");
            // Note: Can't easily format numbers in no_std
            framebuffer::write_str("need 1MB)\nHALTING\n");
            loop {
                unsafe { core::arch::asm!("hlt") };
            }
        }
        None => {
            framebuffer::write_str("ERROR: No conventional memory found\nHALTING\n");
            loop {
                unsafe { core::arch::asm!("hlt") };
            }
        }
    }

    // POST CODE 0x40: Allocator initialized
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x80u16,
            in("al") 0x40u8,
            options(nomem, nostack)
        );
    }

    // Initialize runtime (IDT, PIC, etc)
    framebuffer::write_str("Initializing interrupts...\n");
    unsafe {
        // Initialize exception handlers (IDT)
        if let Err(e) = runtime::init_exception_handlers() {
            framebuffer::write_str("Exception handler init FAILED: ");
            framebuffer::write_str(e);
            framebuffer::write_str("\nHALTING\n");
            loop {
                core::arch::asm!("hlt");
            }
        }

        // Initialize keyboard interrupts (IRQ1)
        if let Err(e) = runtime::init_keyboard_interrupts() {
            framebuffer::write_str("Keyboard init FAILED: ");
            framebuffer::write_str(e);
            framebuffer::write_str("\n");
            // Continue anyway, keyboard may not be connected
        } else {
            framebuffer::write_str("Keyboard initialized\n");
        }

        // Initialize mouse interrupts (IRQ12)
        // DISABLED: Keyboard-only for now (mouse driver needs further testing)
        /*
        if let Err(e) = runtime::init_mouse_interrupts() {
            framebuffer::write_str("Mouse init FAILED: ");
            framebuffer::write_str(e);
            framebuffer::write_str("\n");
            // Continue anyway, mouse may not be connected
        } else {
            framebuffer::write_str("Mouse initialized\n");
        }
        */

        // Enable interrupts so IRQ handlers can work
        core::arch::asm!("sti");
    }
    framebuffer::write_str("Interrupts enabled\n");

    // Jump into shell
    framebuffer::write_str("Starting shell...\n");
    shell::run_shell();

    // Should never return
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
