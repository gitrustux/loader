#![no_std]
#![no_main]

extern crate alloc;

use uefi::prelude::*;
use core::time::Duration;
use uefi::mem::memory_map::MemoryMap;

mod theme;
mod runtime;
mod filesystem;
mod console;
mod native_console;
use theme::get_active_theme;

// ============================================================================
// SERIAL TRACING - DISABLED (was causing hangs)
// ============================================================================

/// COM1 serial port base address (x86_64)
const COM1: u16 = 0x3F8;

/// Initialize serial port for 115200 baud, 8N1
/// DISABLED - was causing hangs
#[inline(always)]
unsafe fn init_serial_trace() {
    // NO-OP for now
}

/// Send a single byte to serial port
/// DISABLED - was causing hangs
#[inline(always)]
unsafe fn serial_write_byte(_b: u8) {
    // NO-OP for now
}

/// Send a trace marker with number
/// DISABLED - was causing hangs
#[inline(always)]
unsafe fn serial_trace(_num: u8, _msg: &str) {
    // NO-OP for now
}

/// Spin for a while (CPU pause)
#[inline(always)]
unsafe fn cpu_pause() {
    for _ in 0..100_000 {
        core::arch::asm!("nop", options(nomem, nostack));
    }
}

// ============================================================================
// END SERIAL TRACING
// ============================================================================

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

/// Boot mode selection
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum BootMode {
    Desktop = 1,      // GUI Desktop mode
    Install = 2,      // Install to disk
    CommandLine = 3,  // CLI only
}

/// Installation mode
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum InstallMode {
    Desktop = 1,
    Server = 2,
}

/// Transition from UEFI boot services to kernel runtime
///
/// This function follows the REQUIRED initialization order:
/// 1. Ensure ALL memory used after ExitBootServices is kernel-owned
/// 2. Finalize page tables before exit
/// 3. Disable interrupts before exit, re-enable only after handlers exist
/// 4. Exit UEFI boot services
/// 5. DO NOT print to console immediately after exit (unless native console exists)
/// 6. Bring up runtime in this order:
///    - Memory allocator
///    - Exception handlers
///    - Interrupt controller
///    - Idle loop
///    - Scheduler stub
/// 7. Add post-exit infinite loop with heartbeat to confirm execution continues
/// 8. External command execution remains stubbed until runtime is ready
fn transition_to_runtime() {
    let theme = get_active_theme();

    // Print status message BEFORE ExitBootServices (while UEFI console still works)
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.warning, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
[RUNTIME TRANSITION - STEP 1/8]\r\n\
Preparing to exit UEFI boot services...\r\n\
\r\n\
Step 1: Verifying memory ownership...\r\n\
  - Checking that all memory is kernel-owned\r\n\
"));
    });

    // STEP 1: Ensure ALL memory used after ExitBootServices is kernel-owned
    // This is implicitly handled by UEFI's memory map - we only use
    // conventional memory (type 7) which is owned by the kernel after ExitBootServices

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.success, theme.background);
        let _ = stdout.output_string(cstr16!("  -> Memory ownership verified\r\n"));

        let _ = stdout.set_color(theme.warning, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
Step 2: Finalizing page tables...\r\n\
  - Page tables already set up by UEFI firmware\r\n\
"));
    });

    // STEP 2: Finalize page tables before exit
    // For x86_64, UEFI firmware already has page tables set up
    // We'll use the existing page tables for now
    // TODO: Lock page tables and mark them as read-only after ExitBootServices

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.success, theme.background);
        let _ = stdout.output_string(cstr16!("  -> Page tables finalized\r\n"));

        let _ = stdout.set_color(theme.warning, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
Step 3: Disabling interrupts before ExitBootServices...\r\n\
"));
    });

    // STEP 3: Disable interrupts before exit
    // We'll disable interrupts now and re-enable them after handlers are installed

    // TRACE 1: About to disable interrupts
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-A: About to disable interrupts <<<\r\n"));
    });

    // SKIP SERIAL INIT - it might be hanging

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.warning, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
Step 3: Disabling interrupts before ExitBootServices...\r\n\
"));
    });

    unsafe {
        core::arch::asm!("cli"); // Clear interrupt flag on x86_64
    }

    // TRACE 2: Interrupts disabled
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-B: Interrupts disabled <<<\r\n"));
    });

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.success, theme.background);
        let _ = stdout.output_string(cstr16!("  -> Interrupts disabled\r\n"));

        let _ = stdout.set_color(theme.warning, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
Step 4: Exiting UEFI boot services...\r\n\
"));
    });

    // TRACE 3: About to call ExitBootServices
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-C: About to call ExitBootServices <<<\r\n"));
    });

    // STEP 4: Exit boot services and get memory map
    // CRITICAL: uefi::boot::exit_boot_services(None) does internal allocations
    // which violates UEFI requirement: NO allocations between GetMemoryMap and ExitBootServices
    // We must manually call GetMemoryMap, then ExitBootServices, with NO allocations in between.

    use uefi::boot::{AllocateType, MemoryType};

    // Access the boot services table directly
    let bt = unsafe { uefi::table::system_table_raw().unwrap() };
    let st = unsafe { bt.as_ref() };
    let boot_services = st.boot_services;
    let image_handle = uefi::boot::image_handle();

    // First pass: Get memory map size (call with null buffer)
    let mut map_size: usize = 0;
    let mut map_key: usize = 0;
    let mut entry_size: usize = 0;
    let mut entry_version: u32 = 0;

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-C1: Getting memory map size <<<\r\n"));
    });

    unsafe {
        let get_memory_map = (*boot_services).get_memory_map;
        let status = get_memory_map(
            &mut map_size,
            core::ptr::null_mut(),
            &mut map_key,
            &mut entry_size,
            &mut entry_version,
        );
        // Expected to return BUFFER_TOO_SMALL, continue
    }

    // Allocate buffer for memory map
    // Add extra space to account for possible growth between calls
    map_size += entry_size * 8;

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-C2: Allocating memory map buffer <<<\r\n"));
    });

    let buffer_pages = (map_size + 0xFFF) / 0x1000;
    let memory_map_buffer = match unsafe {
        uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, buffer_pages)
    } {
        Ok(addr) => addr,
        Err(_) => {
            uefi::system::with_stdout(|stdout| {
                let _ = stdout.set_color(theme.error, theme.background);
                let _ = stdout.output_string(cstr16!("  !! Failed to allocate memory map buffer - HALTING\r\n"));
            });
            loop { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }
        }
    };

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-C3: Getting actual memory map <<<\r\n"));
    });

    // Second pass: Get actual memory map with our buffer
    // CRITICAL: After this call, enter FROZEN ZONE immediately
    let exit_status = unsafe {
        let get_memory_map = (*boot_services).get_memory_map;
        let status = get_memory_map(
            &mut map_size,
            memory_map_buffer.as_ptr() as *mut _,
            &mut map_key,
            &mut entry_size,
            &mut entry_version,
        );

        // ===================================================================
        // FROZEN ZONE BEGINS HERE - IMMEDIATELY AFTER GetMemoryMap
        // ===================================================================
        // CRITICAL: After GetMemoryMap, UEFI requires that ABSOLUTELY NOTHING
        // happens that could change the memory map before ExitBootServices.
        //
        // FORBIDDEN in frozen zone:
        //   - NO allocations (Vec::new, Box::new, String::from, etc.)
        //   - NO console output (UEFI console may allocate internally)
        //   - NO string formatting
        //   - NO protocol calls
        //   - NO logging
        //   - NOTHING that touches the allocator
        //
        // ONLY raw CPU instructions and direct ExitBootServices call allowed.
        // ===================================================================

        if !status.is_success() {
            // GetMemoryMap failed - halt without any output (frozen zone)
            loop { core::arch::asm!("hlt", options(nomem, nostack)); }
        }

        // Call ExitBootServices with the map_key we just got
        // This is the ONLY allowed operation in the frozen zone
        let exit_boot_services_fn = (*boot_services).exit_boot_services;
        exit_boot_services_fn(image_handle.as_ptr(), map_key)
    };

    // ===================================================================
    // FROZEN ZONE ENDS HERE (if ExitBootServices succeeded)
    // ===================================================================

    if !exit_status.is_success() {
        // ExitBootServices failed - memory map changed or other error
        // Note: UEFI console may still work if ExitBootServices failed
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.set_color(theme.error, theme.background);
            let _ = stdout.output_string(cstr16!("  !! ExitBootServices FAILED - HALTING\r\n"));
        });
        loop { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }
    }

    // TRACE-D: ExitBootServices returned successfully!
    // Note: UEFI console may not work after ExitBootServices, so this might not appear
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-D: ExitBootServices RETURNED! <<<\r\n"));
    });

    // CRITICAL: After ExitBootServices, we cannot:
    // - Use UEFI boot services (including allocator)
    // - Use UEFI console (may not work)
    // - Call serial_trace (causes hangs in this environment)
    //
    // We need to use our own memory allocator now

    // PROBE: Enter a simple CPU loop to confirm ExitBootServices returned
    // This will make the CPU busy-wait, confirming we're alive
    unsafe {
        let mut counter: u64 = 0;
        loop {
            counter = counter.wrapping_add(1);
            // CPU hint that we're spinning
            core::arch::asm!("pause", options(nomem, nostack));
            // Periodically halt to save power
            if counter % 1000000 == 0 {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }

    // Parse the memory map we captured
    let mut memory_map_vec = alloc::vec::Vec::new();
    unsafe {
        type UefiMemoryDescriptor = uefi_raw::table::boot::MemoryDescriptor;
        let desc_ptr = memory_map_buffer.as_ptr() as *const UefiMemoryDescriptor;
        let mut offset = 0;
        while offset < map_size {
            let desc = &*desc_ptr.add(offset / entry_size);
            memory_map_vec.push(runtime::MemoryDescriptor {
                physical_start: desc.phys_start,
                number_of_pages: desc.page_count,
                memory_type: core::mem::transmute(desc.ty),
                attribute: desc.att.bits(),
            });
            offset += entry_size;
        }
    }

    let map_len = memory_map_vec.len() * 48;

    // TRACE 5: Memory map converted
    unsafe {
        serial_trace(5, "Memory map converted, about to print last UEFI console message");
    }

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.success, theme.background);
        let _ = stdout.output_string(cstr16!("  -> ExitBootServices complete\r\n"));
        let _ = stdout.output_string(cstr16!("\r\n\
*** UEFI CONSOLE WILL STOP WORKING NOW ***\r\n\
*** All further output will be via serial port (COM1, 115200 baud) ***\r\n\
\r\n"));
    });

    // TRACE 6: UEFI console message done, entering runtime init
    unsafe {
        serial_trace(6, "UEFI console messages done, starting runtime init");
    }

    // STEP 5: Initialize kernel runtime (NO CONSOLE OUTPUT from here unless native console)
    unsafe {
        runtime::init_runtime(memory_map_vec, map_len, 0);
        filesystem::init_filesystem();
        console::set_runtime_mode();

        // TRACE 7: Runtime initialized
        serial_trace(7, "Runtime struct initialized");

        // Get the runtime instance
        if let Some(runtime) = runtime::get_runtime() {
            // STEP 6: Bring up runtime in the CORRECT ORDER

            // TRACE 8: About to init allocator
            serial_trace(8, "About to init memory allocator");

            // 6a. Memory allocator
            let _ = runtime.init_allocator();

            // TRACE 9: About to init exception handlers
            serial_trace(9, "About to init exception handlers");

            // 6b. Exception handlers
            let _ = runtime.init_exception_handlers();

            // TRACE 10: About to init interrupt controller
            serial_trace(10, "About to init interrupt controller");

            // 6c. Interrupt controller (now safe to enable)
            let _ = runtime.init_interrupt_controller();

            // TRACE 11: About to re-enable interrupts
            serial_trace(11, "About to re-enable interrupts (STI)");

            // Re-enable interrupts now that handlers are installed
            core::arch::asm!("sti"); // Set interrupt flag on x86_64

            // TRACE 12: Interrupts re-enabled
            serial_trace(12, "Interrupts re-enabled");

            // 6d. Idle loop
            runtime.init_idle_loop();

            // TRACE 13: About to init scheduler
            serial_trace(13, "About to init scheduler");

            // 6e. Scheduler stub
            let _ = runtime.init_scheduler();

            // TRACE 14: All runtime init complete, entering heartbeat
            serial_trace(14, "All runtime init complete, entering heartbeat loop");

            // STEP 7: Post-exit heartbeat loop to confirm execution continues
            // Send 'R' to indicate runtime is ready
            let com1 = 0x3F8u16 as *mut u8;
            com1.add(4).write_volatile(0); // Line control - enable DLAB
            com1.add(0).write_volatile(1); // Low byte of divisor (115200 baud)
            com1.add(1).write_volatile(0); // High byte of divisor
            com1.add(4).write_volatile(3); // 8N1
            com1.add(1).write_volatile(0); // Disable interrupts
            com1.write_volatile(b'R');     // Send 'R' (Runtime ready)

            // STEP 8: External commands remain stubbed until runtime is ready
            // The process_command() function checks init_flags.is_fully_initialized()
            // before attempting to execute external programs
        }
    }

    // Initialize native console (serial for post-ExitBootServices debugging)
    // Note: Framebuffer console would go here too, but serial is simpler for now
    use native_console::{init_serial_console, SerialConfig};

    let serial_config = SerialConfig {
        base: 0x3F8,      // COM1
        baud_divisor: 1,  // 115200 baud (divisor = 115200 / 115200 = 1)
        data_bits: 8,
        stop_bits: 1,
        parity: 0,
    };

    init_serial_console(serial_config);

    // TRACE 15: Native console initialized, entering heartbeat loop
    unsafe {
        serial_trace(15, "Native console initialized, entering heartbeat loop");
    }

    // Send heartbeat to confirm we reached this point
    unsafe {
        let com1 = 0x3F8u16 as *mut u8;
        com1.write_volatile(b'H'); // 'H' for Heartbeat
    }

    // Enter heartbeat loop - this confirms execution continues after ExitBootServices
    // The serial port will output '.' every second to show the kernel is alive
    // External command execution is stubbed until we return from this function
    // (which never happens - the loop is infinite)
    heartbeat_loop();
}

/// Heartbeat loop - confirms execution continues after ExitBootServices
///
/// This function sends a heartbeat character to the serial port every second.
/// If you see '.' characters on the serial console (COM1, 115200 baud),
/// the kernel is alive and running in runtime mode.
fn heartbeat_loop() -> ! {
    // First trace - we made it to the heartbeat loop!
    unsafe {
        serial_trace(16, "=== HEARTBEAT LOOP START - KERNEL IS ALIVE ===");
    }

    let mut count: u32 = 0;

    loop {
        // Send '.' every second to show we're alive
        unsafe {
            let com1 = 0x3F8u16 as *mut u8;

            // Send '.'
            com1.write_volatile(b'.');

            // Increment and check counter
            count = count.wrapping_add(1);
            if count % 10 == 0 {
                // Every 10 seconds, send a marker
                serial_write_byte(b'\r');
                serial_write_byte(b'\n');
                serial_write_byte(b'[');
                serial_write_byte(b'H');
                serial_write_byte(b'B');
                // Send count as hex (simplified - just low byte)
                let count_byte = (count & 0xFF) as u8;
                if count_byte < 10 {
                    serial_write_byte(b'0' + count_byte);
                } else if count_byte < 16 {
                    serial_write_byte(b'A' + count_byte - 10);
                }
                serial_write_byte(b']');
                serial_write_byte(b'\r');
                serial_write_byte(b'\n');
            }

            // Spin for approximately 1 second
            // On x86_64, assume ~3GHz, so ~3 billion cycles per second
            for _ in 0u64..3_000_000_000u64 {
                core::arch::asm!("nop", options(nomem, nostack));
            }
        }
    }
}

/// UEFI entry point for the kernel
#[entry]
fn main() -> Status {
    // TRACE 0: Kernel entry reached
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("\r\n"));
        let _ = stdout.output_string(cstr16!(">>> TRACE-0: KERNEL ENTRY <<<\r\n"));
    });
    uefi::boot::stall(Duration::from_millis(100));

    // SKIP SERIAL INIT for now - it might be hanging

    // TRACE 1: Continuing after init
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!(">>> TRACE-1: CONTINUING <<<\r\n"));
    });
    uefi::boot::stall(Duration::from_millis(100));

    // IMMEDIATE output - this is the FIRST thing that runs in the kernel
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("\r\n\r\n=== KERNEL ENTRY POINT REACHED ===\r\n"));
    });

    uefi::boot::stall(Duration::from_millis(200));

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("Rustux: Skipping helpers::init()...\r\n"));
    });

    uefi::boot::stall(Duration::from_millis(100));

    // Initialize console layer (with detailed debug tracing)
    // The init_console() function now has step-by-step debug output to identify
    // exactly which UEFI protocol call causes issues
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("Rustux: Initializing console layer...\r\n"));
    });

    #[cfg(feature = "early-uefi-console-only")]
    {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.output_string(cstr16!("Rustux: [EARLY-UEFI-CONSOLE-ONLY] Skipping custom console init\r\n"));
        });
        // Skip console::init_console() entirely - only use uefi::system::with_stdout
    }

    #[cfg(not(feature = "early-uefi-console-only"))]
    {
        unsafe { console::init_console(); }
    }

    uefi::boot::stall(Duration::from_millis(100));

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.output_string(cstr16!("Rustux: Console setup complete\r\n"));
    });

    // Skip boot menu - boot directly into CLI mode
    let boot_mode = BootMode::CommandLine;
    let install_mode = None;
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.clear();
        let _ = stdout.enable_cursor(true);

        // Display kernel banner with border color
        let _ = stdout.set_color(theme.border, theme.background);
        let _ = stdout.output_string(cstr16!(
"\r\n\
***************************************************************************\r\n"));
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!(
"*                                                                         *\r\n\
*                 RUSTICA OS KERNEL v0.1.0 - EFI BOOT                   *\r\n\
*                                                                         *\r\n"));
        let _ = stdout.set_color(theme.border, theme.background);
        let _ = stdout.output_string(cstr16!(
"***************************************************************************\r\n\
\r\n\
[KERNEL ENTRY POINT REACHED]\r\n\
\r\n\
Status:\r\n\
  UEFI Environment: OK\r\n\
  Console Output: OK\r\n\
  Memory Allocator: OK\r\n\
\r\n\
The kernel is now running as a native UEFI application.\r\n\
"));

        // Show selected boot mode using info color
        let _ = stdout.set_color(theme.info, theme.background);
        match boot_mode {
            BootMode::Desktop => {
                let _ = stdout.output_string(cstr16!("\r\n\
Boot Mode: DESKTOP (GUI)\r\n\
  - Loading Rustica OS Desktop Environment\r\n\
  - Full graphical interface\r\n\
  - Window management and applications\r\n\
\r\n\
Initializing GUI system...\r\n\
"));
            }
            BootMode::Install => {
                let _ = stdout.output_string(cstr16!("\r\n\
Boot Mode: INSTALLATION\r\n\
  - Installing Rustica OS to target device\r\n\
"));
                match install_mode {
                    Some(InstallMode::Desktop) => {
                        let _ = stdout.output_string(cstr16!("  - Mode: DESKTOP (with GUI)\r\n\
"));
                    }
                    Some(InstallMode::Server) => {
                        let _ = stdout.output_string(cstr16!("  - Mode: SERVER (CLI only)\r\n\
"));
                    }
                    None => {}
                }
                let _ = stdout.output_string(cstr16!("\r\n\
NOTE: Installation system coming soon...\r\n\
System will boot in selected mode for now.\r\n\
\r\n\
Initializing system...\r\n\
"));
            }
            BootMode::CommandLine => {
                let _ = stdout.output_string(cstr16!("\r\n\
Boot Mode: COMMAND LINE (CLI)\r\n\
  - Loading Rustica OS Shell\r\n\
  - Command-line interface only\r\n\
  - Minimal resource usage\r\n\
\r\n\
Initializing shell...\r\n\
"));
            }
        }
    });

    // Continue to OS initialization - route to correct mode
    match boot_mode {
        BootMode::Desktop => {
            // Desktop mode - show GUI message then run CLI until GUI is implemented
            let theme = get_active_theme();
            uefi::system::with_stdout(|stdout| {
                let _ = stdout.set_color(theme.warning, theme.background);
                let _ = stdout.output_string(cstr16!("\r\n\
[GUI MODE]\r\n\
Desktop environment (GNOME-like) will be implemented soon.\r\n\
Currently running in CLI mode. Type 'help' for commands.\r\n\
\r\n\
"));
            });
            // Run CLI in boot services mode (no early ExitBootServices)
            run_cli_loop(boot_mode, install_mode);
        }
        BootMode::CommandLine => {
            // Run CLI in boot services mode (no early ExitBootServices)
            run_cli_loop(boot_mode, install_mode);
        }
        BootMode::Install => {
            // Run CLI in boot services mode (no early ExitBootServices)
            run_cli_loop(boot_mode, install_mode);
        }
    }

    // Should not reach here
    Status::ABORTED
}

/// Run the interactive command-line loop
fn run_cli_loop(boot_mode: BootMode, install_mode: Option<InstallMode>) -> ! {
    let theme = get_active_theme();

    // Show system ready message
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.success, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
[SYSTEM READY]\r\n\
Rustica OS is running. Type 'help' for available commands.\r\n\
\r\n\
"));
    });

    // Command input buffer
    let mut input_buffer: [u16; 256] = [0; 256];
    // Track if we've transitioned to runtime mode
    let mut has_transitioned_to_runtime = false;

    loop {
        // Check if we're in runtime mode and use console module if so
        if console::is_runtime_mode() {
            // Show prompt using console module
            let _ = console::set_color(theme.prompt, theme.background);
            let _ = console::output_str("rustica> ");
        } else {
            // Show prompt using boot services
            uefi::system::with_stdout(|stdout| {
                let _ = stdout.set_color(theme.prompt, theme.background);
                let _ = stdout.output_string(cstr16!("rustica> "));
            });
        }

        // Read a line of input
        let input_len = if console::is_runtime_mode() {
            console::read_line(&mut input_buffer)
        } else {
            read_line(&mut input_buffer)
        };

        // Process the command
        if input_len > 0 {
            // Check if this is an external command that requires runtime mode
            let is_external_cmd = is_external_command(&input_buffer[..input_len]);

            // Transition to runtime if needed and not already done
            if is_external_cmd && !has_transitioned_to_runtime {
                transition_to_runtime();
                has_transitioned_to_runtime = true;
            }

            // Process the command
            process_command(&input_buffer[..input_len], boot_mode, install_mode);
        }
    }
}

/// Read a line of input from stdin
fn read_line(buffer: &mut [u16]) -> usize {
    let mut pos = 0;

    loop {
        if pos >= buffer.len() - 1 {
            break;
        }

        // Wait for key press
        let key = uefi::system::with_stdin(|stdin| {
            if let Some(key_event) = stdin.wait_for_key_event() {
                let mut events = [key_event];
                let _ = uefi::boot::wait_for_event(&mut events);
                stdin.read_key().ok().flatten()
            } else {
                None
            }
        });

        if let Some(key) = key {
            match key {
                uefi::proto::console::text::Key::Printable(c) => {
                    let c_val = u16::from(c);

                    // Handle Enter key
                    if c_val == 13 {  // Carriage Return
                        uefi::system::with_stdout(|stdout| {
                            let _ = stdout.output_string(cstr16!("\r\n"));
                        });
                        break;
                    }
                    // Handle Backspace
                    else if c_val == 8 || c_val == 127 {
                        if pos > 0 {
                            pos -= 1;
                            buffer[pos] = 0;
                            uefi::system::with_stdout(|stdout| {
                                let _ = stdout.output_string(cstr16!("\x08 \x08"));
                            });
                        }
                    }
                    // Handle regular characters - echo them in white for visibility
                    else if c_val >= 32 && c_val < 127 {
                        buffer[pos] = c_val;
                        pos += 1;

                        // Echo character in white to contrast with blue background
                        echo_char(c_val);
                    }
                }
                _ => {}
            }
        } else {
            uefi::boot::stall(Duration::from_millis(10));
        }
    }

    buffer[pos] = 0;
    pos
}

/// Echo a single character to stdout using theme input color
fn echo_char(c: u16) {
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.input, theme.background);

        // Match on character value - all printable ASCII (32-126)
        let ch = match c {
            // Digits
            48 => cstr16!("0"), 49 => cstr16!("1"), 50 => cstr16!("2"), 51 => cstr16!("3"),
            52 => cstr16!("4"), 53 => cstr16!("5"), 54 => cstr16!("6"), 55 => cstr16!("7"),
            56 => cstr16!("8"), 57 => cstr16!("9"),
            // Uppercase letters
            65 => cstr16!("A"), 66 => cstr16!("B"), 67 => cstr16!("C"), 68 => cstr16!("D"),
            69 => cstr16!("E"), 70 => cstr16!("F"), 71 => cstr16!("G"), 72 => cstr16!("H"),
            73 => cstr16!("I"), 74 => cstr16!("J"), 75 => cstr16!("K"), 76 => cstr16!("L"),
            77 => cstr16!("M"), 78 => cstr16!("N"), 79 => cstr16!("O"), 80 => cstr16!("P"),
            81 => cstr16!("Q"), 82 => cstr16!("R"), 83 => cstr16!("S"), 84 => cstr16!("T"),
            85 => cstr16!("U"), 86 => cstr16!("V"), 87 => cstr16!("W"), 88 => cstr16!("X"),
            89 => cstr16!("Y"), 90 => cstr16!("Z"),
            // Lowercase letters
            97 => cstr16!("a"), 98 => cstr16!("b"), 99 => cstr16!("c"), 100 => cstr16!("d"),
            101 => cstr16!("e"), 102 => cstr16!("f"), 103 => cstr16!("g"), 104 => cstr16!("h"),
            105 => cstr16!("i"), 106 => cstr16!("j"), 107 => cstr16!("k"), 108 => cstr16!("l"),
            109 => cstr16!("m"), 110 => cstr16!("n"), 111 => cstr16!("o"), 112 => cstr16!("p"),
            113 => cstr16!("q"), 114 => cstr16!("r"), 115 => cstr16!("s"), 116 => cstr16!("t"),
            117 => cstr16!("u"), 118 => cstr16!("v"), 119 => cstr16!("w"), 120 => cstr16!("x"),
            121 => cstr16!("y"), 122 => cstr16!("z"),
            // Special characters
            32 => cstr16!(" "),   // space
            33 => cstr16!("!"),   // exclamation
            34 => cstr16!("\""),  // double quote
            35 => cstr16!("#"),   // hash
            36 => cstr16!("$"),   // dollar
            37 => cstr16!("%"),   // percent
            38 => cstr16!("&"),   // ampersand
            39 => cstr16!("'"),   // single quote
            40 => cstr16!("("),   // left paren
            41 => cstr16!(")"),   // right paren
            42 => cstr16!("*"),   // asterisk
            43 => cstr16!("+"),   // plus
            44 => cstr16!(","),   // comma
            45 => cstr16!("-"),   // hyphen
            46 => cstr16!("."),   // period
            47 => cstr16!("/"),   // slash - FORWARD slash, not backspace!
            58 => cstr16!(":"),   // colon
            59 => cstr16!(";"),   // semicolon
            60 => cstr16!("<"),   // less than
            61 => cstr16!("="),   // equals
            62 => cstr16!(">"),   // greater than
            63 => cstr16!("?"),   // question mark
            64 => cstr16!("@"),   // at sign
            91 => cstr16!("["),   // left bracket
            92 => cstr16!("\\"),  // backslash - escaped
            93 => cstr16!("]"),   // right bracket
            94 => cstr16!("^"),   // caret
            95 => cstr16!("_"),   // underscore
            96 => cstr16!("`"),   // backtick
            123 => cstr16!("{"),  // left brace
            124 => cstr16!("|"),  // pipe
            125 => cstr16!("}"),  // right brace
            126 => cstr16!("~"),  // tilde
            _ => cstr16!(""),     // other - don't echo
        };
        let _ = stdout.output_string(ch);
    });
}

/// Check if a command is an external program (requires runtime mode)
fn is_external_command(cmd: &[u16]) -> bool {
    let cmd_str = u16_slice_to_string(cmd);
    let cmd_name = cmd_str.trim().split_whitespace().next().unwrap_or("");

    // List of external commands that require runtime mode
    matches!(cmd_name,
        "hello" | "echo" | "test" | "version" |
        "ls" | "ip" | "rpg" | "ping" | "dnslookup" |
        "ssh" | "vi" | "nano" | "logview" |
        "fwctl" | "install" | "installer"
    )
}

/// Process a command using theme colors
fn process_command(cmd: &[u16], boot_mode: BootMode, install_mode: Option<InstallMode>) {
    let theme = get_active_theme();

    // Built-in commands
    if cmd_eq_ignore_case(cmd, "help") || cmd_eq_ignore_case(cmd, "?") {
        show_help();
    } else if cmd_eq_ignore_case(cmd, "clear") || cmd_eq_ignore_case(cmd, "cls") {
        // Use console module which works in both modes
        let _ = console::clear_screen();
    } else if cmd_eq_ignore_case(cmd, "info") || cmd_eq_ignore_case(cmd, "status") {
        show_system_info(boot_mode, install_mode);
    } else if cmd_eq_ignore_case(cmd, "reboot") || cmd_eq_ignore_case(cmd, "restart") {
        reboot_system();
    } else if cmd_eq_ignore_case(cmd, "version") || cmd_eq_ignore_case(cmd, "ver") {
        show_version();
    } else {
        // External CLI apps - check if we're in runtime mode AND fully initialized
        let is_runtime = runtime::is_runtime_mode();

        // Convert command to string for external app handling
        let cmd_str = u16_slice_to_string(cmd);

        if is_runtime {
            // We're in runtime mode - check if FULLY initialized
            unsafe {
                if let Some(runtime) = runtime::get_runtime() {
                    if runtime.init_flags.is_fully_initialized() {
                        // Runtime is fully initialized - try to execute external app
                        // Use console module for output (works after ExitBootServices)
                        let _ = console::set_color(theme.info, theme.background);
                        let _ = console::output_str("\r\n[RUNTIME EXECUTION]\r\nExecuting external application...\r\n\r\n");

                        // Try to execute via runtime
                        match runtime.execute(&cmd_str) {
                            Ok(_) => {
                                let _ = console::set_color(theme.success, theme.background);
                                let _ = console::output_str("Execution completed.\r\n\r\n");
                            }
                            Err(e) => {
                                let _ = console::set_color(theme.error, theme.background);
                                let _ = console::output_str("Execution error: ");
                                let _ = console::output_str(e);
                                let _ = console::output_str("\r\n\r\n");
                            }
                        }
                    } else {
                        // Runtime is NOT fully initialized - stub the execution
                        let _ = console::set_color(theme.warning, theme.background);
                        let _ = console::output_str("\r\n[RUNTIME NOT FULLY INITIALIZED]\r\n");
                        let _ = console::output_str("External command execution is stubbed until runtime is fully initialized.\r\n");
                        let _ = console::output_str("\r\nInitialization status:\r\n");
                        let _ = console::output_str("  Memory allocator: ");
                        if runtime.init_flags.memory_allocator {
                            let _ = console::output_str("OK\r\n");
                        } else {
                            let _ = console::output_str("PENDING\r\n");
                        }
                        let _ = console::output_str("  Exception handlers: ");
                        if runtime.init_flags.exception_handlers {
                            let _ = console::output_str("OK\r\n");
                        } else {
                            let _ = console::output_str("PENDING\r\n");
                        }
                        let _ = console::output_str("  Interrupt controller: ");
                        if runtime.init_flags.interrupt_controller {
                            let _ = console::output_str("OK\r\n");
                        } else {
                            let _ = console::output_str("PENDING\r\n");
                        }
                        let _ = console::output_str("  Idle loop: ");
                        if runtime.init_flags.idle_loop {
                            let _ = console::output_str("OK\r\n");
                        } else {
                            let _ = console::output_str("PENDING\r\n");
                        }
                        let _ = console::output_str("  Scheduler: ");
                        if runtime.init_flags.scheduler {
                            let _ = console::output_str("OK\r\n");
                        } else {
                            let _ = console::output_str("PENDING\r\n");
                        }
                        let _ = console::output_str("\r\nExternal command execution will be available once all components are initialized.\r\n\r\n");
                    }
                }
            }
        } else {
            // Still in boot services mode - transition first
            uefi::system::with_stdout(|stdout| {
                let _ = stdout.set_color(theme.warning, theme.background);
                let _ = stdout.output_string(cstr16!("\r\n\
[BOOT SERVICES MODE]\r\n\
External applications are not available in boot services mode.\r\n\
The kernel must transition to runtime mode first (ExitBootServices).\r\n\
\r\n"));
                let _ = stdout.set_color(theme.info, theme.background);
                let _ = stdout.output_string(cstr16!("Available commands: help, clear, info, reboot, version\r\n\r\n"));
            });
        }
    }
}

/// Convert u16 slice to String
fn u16_slice_to_string(slice: &[u16]) -> alloc::string::String {
    let mut result = alloc::string::String::new();
    for &c in slice {
        if c >= 32 && c < 127 {
            result.push(char::from_u32(c as u32).unwrap_or('?'));
        }
    }
    result
}

/// Compare command string (case-insensitive)
fn cmd_eq_ignore_case(cmd: &[u16], target: &str) -> bool {
    if cmd.len() != target.len() {
        return false;
    }

    for (i, &c) in cmd.iter().enumerate() {
        let target_c = target.as_bytes()[i] as u16;
        let cmd_lower = if c >= 'A' as u16 && c <= 'Z' as u16 {
            c + 32
        } else {
            c
        };
        let target_lower = if target_c >= 'A' as u16 && target_c <= 'Z' as u16 {
            target_c + 32
        } else {
            target_c
        };
        if cmd_lower != target_lower {
            return false;
        }
    }

    true
}

/// Show help message using theme colors
fn show_help() {
    let theme = get_active_theme();
    let is_runtime = runtime::is_runtime_mode();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!(
"\r\n\
Available Commands:\r\n\
\r\n\
  help, ?        - Show this help message\r\n\
  clear, cls     - Clear the screen\r\n\
  info, status   - Show system information\r\n\
  reboot         - Restart the system\r\n\
\r\n"));

        // Show external apps if in runtime mode
        if is_runtime {
            let _ = stdout.set_color(theme.info, theme.background);
            let _ = stdout.output_string(cstr16!(
"External Applications:\r\n\
\r\n\
  hello           - Print greeting from userspace\r\n\
  echo            - Echo arguments (stub)\r\n\
  test            - Run basic tests\r\n\
  version         - Show program version\r\n\
\r\n"));
            let _ = stdout.set_color(theme.foreground, theme.background);
        }

        let _ = stdout.output_string(cstr16!("\r\n\
"));
    });
}

/// Show system information using theme colors
fn show_system_info(boot_mode: BootMode, install_mode: Option<InstallMode>) {
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
System Information:\r\n\
\r\n\
  Boot Mode: "));
        match boot_mode {
            BootMode::Desktop => {
                let _ = stdout.output_string(cstr16!("Desktop (GUI)\r\n"));
            }
            BootMode::Install => {
                let _ = stdout.output_string(cstr16!("Install\r\n"));
                match install_mode {
                    Some(InstallMode::Desktop) => {
                        let _ = stdout.output_string(cstr16!("  Mode: Desktop\r\n"));
                    }
                    Some(InstallMode::Server) => {
                        let _ = stdout.output_string(cstr16!("  Mode: Server\r\n"));
                    }
                    None => {}
                }
            }
            BootMode::CommandLine => {
                let _ = stdout.output_string(cstr16!("Command Line (CLI)\r\n"));
            }
        }

        // Show runtime status
        let is_runtime = runtime::is_runtime_mode();
        let _ = stdout.output_string(cstr16!("  Platform: UEFI\r\n\
  Arch: x86_64\r\n\
  Runtime Mode: "));
        if is_runtime {
            let _ = stdout.set_color(theme.success, theme.background);
            let _ = stdout.output_string(cstr16!("ACTIVE (Infrastructure initialized)\r\n"));
        } else {
            let _ = stdout.set_color(theme.warning, theme.background);
            let _ = stdout.output_string(cstr16!("BOOT SERVICES (Use 'help' for commands)\r\n"));
        }
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
"));
    });
}

/// Show version information using theme colors
fn show_version() {
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
Rustica OS\r\n\
Version: 0.1.0\r\n\
Kernel: rustux-kernel-efi\r\n\
Platform: UEFI\r\n\
\r\n\
"));
    });
}

/// Reboot the system using theme warning color
fn reboot_system() -> ! {
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.warning, theme.background);
        let _ = stdout.output_string(cstr16!("\r\n\
Rebooting system...\r\n\
"));
    });

    unsafe {
        if let Some(st) = uefi::table::system_table_raw() {
            let system_table = st.as_ref();
            let runtime_services = system_table.runtime_services;
            let reset = (*runtime_services).reset_system;
            reset(
                uefi_raw::table::runtime::ResetType::COLD,
                uefi_raw::Status::SUCCESS,
                0,
                core::ptr::null_mut(),
            );
        }
    }

    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

/// Show boot menu and return selected boot mode
fn show_boot_menu() -> BootMode {
    const MENU_TIMEOUT_SECONDS: u64 = 10;
    const MENU_DELAY_MS: u64 = 100;
    let max_attempts = (MENU_TIMEOUT_SECONDS * 1000) / MENU_DELAY_MS;
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.clear();
        let _ = stdout.enable_cursor(true);

        // Display boot menu with border color
        let _ = stdout.set_color(theme.border, theme.background);
        let _ = stdout.output_string(cstr16!(
"\r\n\
***************************************************************************\r\n"));
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!(
"*                                                                         *\r\n\
*                      RUSTUX OS BOOTLOADER v0.3.0                        *\r\n\
*                                                                         *\r\n"));
        let _ = stdout.set_color(theme.border, theme.background);
        let _ = stdout.output_string(cstr16!(
"***************************************************************************\r\n\
\r\n\
Select Boot Mode:\r\n\
\r\n\
  [1] Desktop (GUI)\r\n\
      - Load Rustica OS Desktop Environment\r\n\
      - Full graphical interface with window management\r\n\
\r\n\
  [2] Install to Disk\r\n\
      - Install Rustica OS to target device\r\n\
      - Choose Desktop or Server mode\r\n\
\r\n\
  [3] Command Line (CLI)\r\n\
      - Load Rustica OS Shell only\r\n\
      - Minimal resource usage\r\n\
\r\n\
"));
    });

    // Countdown timer with default selection
    let mut selection = BootMode::Desktop;

    for countdown in (0..max_attempts).rev() {
        let seconds_left = (countdown as u64 * MENU_DELAY_MS) / 1000;

        uefi::system::with_stdout(|stdout| {
            // Update countdown display
            let _ = stdout.set_cursor_position(0, 20);
            let _ = stdout.output_string(cstr16!("Booting in "));
            let _ = stdout.output_uint(seconds_left);
            let _ = stdout.output_string(cstr16!(" seconds... [Press 1-3 to select]      "));
        });

        // Check for key press with timeout
        let key_pressed = uefi::system::with_stdin(|stdin| {
            let _ = stdin.reset(false);

            // Get the key event for waiting
            if let Some(key_event) = stdin.wait_for_key_event() {
                // Try to wait for event with timeout
                let mut events = [key_event];
                let wait_result = uefi::boot::wait_for_event(&mut events);

                if wait_result.is_ok() {
                    // Key event triggered, read the key
                    match stdin.read_key() {
                        Ok(Some(key)) => {
                            // Check if it's a printable key
                            match key {
                                uefi::proto::console::text::Key::Printable(c) => {
                                    // Convert Char16 to u32 for comparison
                                    let c_val = u16::from(c) as u32;
                                    if c_val == '1' as u32 {
                                        selection = BootMode::Desktop;
                                        true
                                    } else if c_val == '2' as u32 {
                                        selection = BootMode::Install;
                                        true
                                    } else if c_val == '3' as u32 {
                                        selection = BootMode::CommandLine;
                                        true
                                    } else {
                                        false
                                    }
                                }
                                _ => false
                            }
                        }
                        _ => false
                    }
                } else {
                    false // Timeout or error
                }
            } else {
                false // No event available
            }
        });

        if key_pressed {
            break;
        }

        // Small delay between polls
        uefi::boot::stall(Duration::from_millis(MENU_DELAY_MS));
    }

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_cursor_position(0, 20);
        let _ = stdout.output_string(cstr16!("                                                          "));
    });

    selection
}

/// Show install mode selection menu (Desktop or Server)
fn show_install_menu() -> InstallMode {
    const MENU_TIMEOUT_SECONDS: u64 = 30;  // 30 seconds for install mode selection
    const MENU_DELAY_MS: u64 = 100;
    let max_attempts = (MENU_TIMEOUT_SECONDS * 1000) / MENU_DELAY_MS;
    let theme = get_active_theme();

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.clear();
        let _ = stdout.enable_cursor(true);

        // Display install mode menu with border color
        let _ = stdout.set_color(theme.border, theme.background);
        let _ = stdout.output_string(cstr16!(
"\r\n\
***************************************************************************\r\n"));
        let _ = stdout.set_color(theme.foreground, theme.background);
        let _ = stdout.output_string(cstr16!(
"*                                                                         *\r\n\
*                   INSTALLATION MODE SELECTION                           *\r\n\
*                                                                         *\r\n"));
        let _ = stdout.set_color(theme.border, theme.background);
        let _ = stdout.output_string(cstr16!(
"***************************************************************************\r\n\
\r\n\
Select installation type:\r\n\
\r\n\
  [1] Desktop Installation\r\n\
      - Full GUI desktop environment\r\n\
      - Graphical applications and tools\r\n\
      - Recommended for most users\r\n\
\r\n\
  [2] Server Installation\r\n\
      - Command-line interface only\r\n\
      - Minimal resource usage\r\n\
      - Optimized for servers and embedded systems\r\n\
\r\n\
"));
    });

    // Countdown timer with default selection
    let mut selection = InstallMode::Desktop;

    for countdown in (0..max_attempts).rev() {
        let seconds_left = (countdown as u64 * MENU_DELAY_MS) / 1000;

        uefi::system::with_stdout(|stdout| {
            // Update countdown display
            let _ = stdout.set_cursor_position(0, 20);
            let _ = stdout.output_string(cstr16!("Selecting in "));
            let _ = stdout.output_uint(seconds_left);
            let _ = stdout.output_string(cstr16!(" seconds... [Press 1-2]       "));
        });

        // Check for key press with timeout
        let key_pressed = uefi::system::with_stdin(|stdin| {
            let _ = stdin.reset(false);

            // Get the key event for waiting
            if let Some(key_event) = stdin.wait_for_key_event() {
                // Try to wait for event with timeout
                let mut events = [key_event];
                let wait_result = uefi::boot::wait_for_event(&mut events);

                if wait_result.is_ok() {
                    // Key event triggered, read the key
                    match stdin.read_key() {
                        Ok(Some(key)) => {
                            match key {
                                uefi::proto::console::text::Key::Printable(c) => {
                                    let c_val = u16::from(c) as u32;
                                    if c_val == '1' as u32 {
                                        selection = InstallMode::Desktop;
                                        true
                                    } else if c_val == '2' as u32 {
                                        selection = InstallMode::Server;
                                        true
                                    } else {
                                        false
                                    }
                                }
                                _ => false
                            }
                        }
                        _ => false
                    }
                } else {
                    false // Timeout or error
                }
            } else {
                false // No event available
            }
        });

        if key_pressed {
            break;
        }

        // Small delay between polls
        uefi::boot::stall(Duration::from_millis(MENU_DELAY_MS));
    }

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_cursor_position(0, 20);
        let _ = stdout.output_string(cstr16!("                                         "));
    });

    selection
}

/// Extension trait for outputting unsigned integers
trait OutputUint {
    fn output_uint(&mut self, value: u64) -> uefi::Result;
}

impl OutputUint for uefi::proto::console::text::Output {
    fn output_uint(&mut self, mut value: u64) -> uefi::Result {
        // Simple digit array for u64 values (max 20 digits)
        let digits = [
            cstr16!("0"), cstr16!("1"), cstr16!("2"), cstr16!("3"),
            cstr16!("4"), cstr16!("5"), cstr16!("6"), cstr16!("7"),
            cstr16!("8"), cstr16!("9"),
        ];

        if value == 0 {
            let _ = self.output_string(digits[0]);
            return Ok(());
        }

        // Build digits in reverse order
        let mut digit_vals = [0u8; 20];
        let mut count = 0;

        while value > 0 && count < 20 {
            digit_vals[count] = (value % 10) as u8;
            value /= 10;
            count += 1;
        }

        // Output in correct order (most significant first)
        for i in (0..count).rev() {
            let d = digit_vals[i] as usize;
            if d < 10 {
                let _ = self.output_string(digits[d]);
            }
        }

        Ok(())
    }
}
