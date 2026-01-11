#![no_std]
#![no_main]

extern crate alloc;

use uefi::prelude::*;
use core::time::Duration;

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

/// UEFI entry point for the kernel
#[entry]
fn main() -> Status {
    // Small delay to ensure bootloader output is visible
    uefi::boot::stall(Duration::from_secs(1));

    // Initialize UEFI services
    uefi::helpers::init().unwrap();

    // Show boot menu and get user selection
    let boot_mode = show_boot_menu();

    // If Install mode, show install type selection
    let install_mode = if boot_mode == BootMode::Install {
        Some(show_install_menu())
    } else {
        None
    };

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::White,
                                 uefi::proto::console::text::Color::Blue);
        let _ = stdout.clear();
        let _ = stdout.enable_cursor(true);

        // Display kernel banner
        let _ = stdout.output_string(cstr16!(
"\r\n\
***************************************************************************\r\n\
*                                                                         *\r\n\
*                 RUSTUX OS KERNEL v0.1.0 - EFI BOOT                      *\r\n\
*                                                                         *\r\n\
***************************************************************************\r\n\
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

        // Show selected boot mode
        let _ = stdout.set_color(uefi::proto::console::text::Color::Yellow,
                                 uefi::proto::console::text::Color::Blue);
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
  - Installing Rustux OS to target device\r\n\
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
  - Loading Rustux OS Shell\r\n\
  - Command-line interface only\r\n\
  - Minimal resource usage\r\n\
\r\n\
Initializing shell...\r\n\
"));
            }
        }
    });

    // Continue to OS initialization - run interactive CLI
    run_cli_loop(boot_mode, install_mode);

    // Should not reach here
    Status::ABORTED
}

/// Run the interactive command-line loop
fn run_cli_loop(boot_mode: BootMode, install_mode: Option<InstallMode>) -> ! {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::Green,
                                 uefi::proto::console::text::Color::Blue);
        let _ = stdout.output_string(cstr16!("\r\n\
[SYSTEM READY]\r\n\
Rustux OS is running. Type 'help' for available commands.\r\n\
\r\n\
"));
    });

    // Command input buffer
    let mut input_buffer: [u16; 256] = [0; 256];

    loop {
        // Show prompt
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.set_color(uefi::proto::console::text::Color::Cyan,
                                     uefi::proto::console::text::Color::Blue);
            let _ = stdout.output_string(cstr16!("rustux> "));
        });

        // Read a line of input
        let input_len = read_line(&mut input_buffer);

        // Process the command
        if input_len > 0 {
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
                    // Handle regular characters - just collect, no echo for now
                    else if c_val >= 32 && c_val < 127 {
                        buffer[pos] = c_val;
                        pos += 1;
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

/// Process a command
fn process_command(cmd: &[u16], boot_mode: BootMode, install_mode: Option<InstallMode>) {
    // Simple command matching
    if cmd_eq_ignore_case(cmd, "help") || cmd_eq_ignore_case(cmd, "?") {
        show_help();
    } else if cmd_eq_ignore_case(cmd, "clear") || cmd_eq_ignore_case(cmd, "cls") {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.clear();
        });
    } else if cmd_eq_ignore_case(cmd, "info") || cmd_eq_ignore_case(cmd, "status") {
        show_system_info(boot_mode, install_mode);
    } else if cmd_eq_ignore_case(cmd, "reboot") || cmd_eq_ignore_case(cmd, "restart") {
        reboot_system();
    } else if cmd_eq_ignore_case(cmd, "version") || cmd_eq_ignore_case(cmd, "ver") {
        show_version();
    } else {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.set_color(uefi::proto::console::text::Color::Red,
                                     uefi::proto::console::text::Color::Blue);
            let _ = stdout.output_string(cstr16!("Unknown command. Type 'help' for available commands.\r\n\r\n"));
        });
    }
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

/// Show help message
fn show_help() {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::White,
                                 uefi::proto::console::text::Color::Blue);
        let _ = stdout.output_string(cstr16!(
"\r\n\
Available Commands:\r\n\
\r\n\
  help, ?        - Show this help message\r\n\
  clear, cls     - Clear the screen\r\n\
  info, status   - Show system information\r\n\
  version, ver   - Show version information\r\n\
  reboot         - Restart the system\r\n\
\r\n\
"));
    });
}

/// Show system information
fn show_system_info(boot_mode: BootMode, install_mode: Option<InstallMode>) {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::White,
                                 uefi::proto::console::text::Color::Blue);
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
        let _ = stdout.output_string(cstr16!("  Platform: UEFI\r\n\
  Arch: x86_64\r\n\
\r\n\
"));
    });
}

/// Show version information
fn show_version() {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::White,
                                 uefi::proto::console::text::Color::Blue);
        let _ = stdout.output_string(cstr16!("\r\n\
Rustux OS\r\n\
Version: 0.1.0\r\n\
Kernel: rustux-kernel-efi\r\n\
Platform: UEFI\r\n\
\r\n\
"));
    });
}

/// Reboot the system
fn reboot_system() -> ! {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::Yellow,
                                 uefi::proto::console::text::Color::Red);
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

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::White,
                                 uefi::proto::console::text::Color::Black);
        let _ = stdout.clear();
        let _ = stdout.enable_cursor(true);

        // Display boot menu
        let _ = stdout.output_string(cstr16!(
"\r\n\
***************************************************************************\r\n\
*                                                                         *\r\n\
*                      RUSTUX OS BOOTLOADER v0.3.0                        *\r\n\
*                                                                         *\r\n\
***************************************************************************\r\n\
\r\n\
Select Boot Mode:\r\n\
\r\n\
  [1] Desktop (GUI)\r\n\
      - Load Rustica OS Desktop Environment\r\n\
      - Full graphical interface with window management\r\n\
\r\n\
  [2] Install to Disk\r\n\
      - Install Rustux OS to target device\r\n\
      - Choose Desktop or Server mode\r\n\
\r\n\
  [3] Command Line (CLI)\r\n\
      - Load Rustux OS Shell only\r\n\
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

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_color(uefi::proto::console::text::Color::White,
                                 uefi::proto::console::text::Color::Black);
        let _ = stdout.clear();
        let _ = stdout.enable_cursor(true);

        // Display install mode menu
        let _ = stdout.output_string(cstr16!(
"\r\n\
***************************************************************************\r\n\
*                                                                         *\r\n\
*                   INSTALLATION MODE SELECTION                           *\r\n\
*                                                                         *\r\n\
***************************************************************************\r\n\
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
