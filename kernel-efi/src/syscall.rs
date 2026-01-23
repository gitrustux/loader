// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! System Call Interface (Rustux ABI)
//!
//! This module provides the system call interface for userspace programs.
//! It implements a minimal set of syscalls: sys_write, sys_read, and sys_exit.
//!
//! ## Syscall ABI (Rustux)
//! - Syscall number: rax
//! - Arguments: rdi, rsi, rdx
//! - Return value: rax
//! - Instruction: syscall
//!
//! ## Syscall Numbers
//! - sys_write: 1 (write to stdout)
//! - sys_read: 2 (read from stdin)

#![allow(dead_code)] // Many items are for future features
//! - sys_exit: 60 (terminate process)
//!
//! ## Usage
//! ```rust
//! // Userspace program:
//! #[inline(always)]
//! fn syscall_write(fd: u64, buf: *const u8, len: u64) {
//!     unsafe {
//!         core::arch::asm!(
//!             "syscall",
//!             in("rax") 1u64,
//!             in("rdi") fd,
//!             in("rsi") buf,
//!             in("rdx") len
//!         );
//!     }
//! }
//! ```

/// Syscall numbers
pub const SYS_WRITE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_EXIT: u64 = 60;

/// File descriptors
pub const STDIN_FD: u64 = 0;
pub const STDOUT_FD: u64 = 1;
pub const STDERR_FD: u64 = 2;

/// Syscall handler function pointer type
type SyscallHandler = unsafe extern "C" fn(u64, u64, u64) -> u64;

/// Maximum number of syscalls
const MAX_SYSCALLS: usize = 128;

/// Syscall dispatch table
static mut SYSCALL_TABLE: [Option<SyscallHandler>; MAX_SYSCALLS] = [None; MAX_SYSCALLS];

/// Initialize the syscall table
pub fn init() {
    unsafe {
        // Register syscalls
        SYSCALL_TABLE[SYS_WRITE as usize] = Some(sys_write_handler);
        SYSCALL_TABLE[SYS_READ as usize] = Some(sys_read_handler);
        SYSCALL_TABLE[SYS_EXIT as usize] = Some(sys_exit_handler);
    }
}

/// Syscall entry point (called from assembly stub)
///
/// # Arguments
/// * `syscall_number` - Syscall number (from rax)
/// * `arg1` - First argument (from rdi)
/// * `arg2` - Second argument (from rsi)
/// * `arg3` - Third argument (from rdx)
///
/// # Returns
/// * Syscall return value (placed in rax)
#[no_mangle]
pub extern "C" fn syscall_entry(syscall_number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    unsafe {
        // Check if syscall number is valid
        if (syscall_number as usize) < MAX_SYSCALLS {
            if let Some(handler) = SYSCALL_TABLE[syscall_number as usize] {
                // Call the syscall handler
                return handler(arg1, arg2, arg3);
            }
        }

        // Invalid syscall number
        return !0u64; // Return -1 (as u64)
    }
}

/// sys_write syscall handler
///
/// Writes data to the console (stdout/stderr).
///
/// # Arguments
/// * `fd` - File descriptor (must be STDOUT_FD or STDERR_FD)
/// * `buffer` - Pointer to data to write
/// * `length` - Number of bytes to write
///
/// # Returns
/// * Number of bytes written, or !0 on error
unsafe extern "C" fn sys_write_handler(fd: u64, buffer: u64, length: u64) -> u64 {
    // Validate file descriptor
    if fd != STDOUT_FD && fd != STDERR_FD {
        return !0; // EBADF
    }

    // Validate buffer pointer
    if buffer == 0 {
        return !0; // EFAULT
    }

    // Write each character to the VGA console
    let buf_ptr = buffer as *const u8;
    let mut written = 0u64;

    for i in 0..length {
        let c = *buf_ptr.add(i as usize) as char;
        crate::vga_console::putc(c);
        written += 1;
    }

    written
}

/// sys_read syscall handler
///
/// Reads data from the keyboard (stdin).
///
/// # Arguments
/// * `fd` - File descriptor (must be STDIN_FD)
/// * `buffer` - Pointer to buffer to read into
/// * `length` - Maximum number of bytes to read
///
/// # Returns
/// * Number of bytes read, or !0 on error
unsafe extern "C" fn sys_read_handler(fd: u64, buffer: u64, length: u64) -> u64 {
    // Validate file descriptor
    if fd != STDIN_FD {
        return !0; // EBADF
    }

    // Validate buffer pointer
    if buffer == 0 {
        return !0; // EFAULT
    }

    // Validate length
    if length == 0 {
        return 0; // EOF
    }

    // Read from keyboard into buffer
    let buf_ptr = buffer as *mut u8;
    let buf_slice = core::slice::from_raw_parts_mut(buf_ptr, length as usize);

    // Read a line from keyboard (blocks until Enter)
    let bytes_read = crate::keyboard::read_line(buf_slice);

    bytes_read as u64
}

/// sys_exit syscall handler
///
/// Terminates the current process.
///
/// # Arguments
/// * `exit_code` - Exit code
///
/// # Returns
/// * Does not return
unsafe extern "C" fn sys_exit_handler(exit_code: u64, _arg2: u64, _arg3: u64) -> u64 {
    // Write exit message to console
    crate::vga_console::set_color(14, 0); // Yellow on black
    crate::vga_console::puts("\n*** PROCESS EXITED ***\n");
    crate::vga_console::puts("Exit code: ");
    crate::vga_console::put_hex(exit_code);
    crate::vga_console::puts("\n");

    // TODO: Implement process cleanup
    // For now, just halt
    loop {
        core::arch::asm!("hlt", options(nomem, nostack));
    }
}

/// Syscall interrupt stub (IDT vector 0x80)
///
/// This stub is called when a userspace program executes the `syscall` instruction.
/// It saves all registers, calls the syscall handler, and returns.
#[no_mangle]
#[unsafe(naked)]
pub extern "C" fn syscall_stub() -> ! {
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

        // Save syscall arguments (before they get clobbered)
        // rax = syscall number, rdi = arg1, rsi = arg2, rdx = arg3
        "mov r10, rdx",  // Save arg3
        "mov r11, rsi",  // Save arg2
        "mov r12, rdi",  // Save arg1

        // Call syscall_entry(syscall_number, arg1, arg2, arg3)
        "mov rdi, rax",  // Syscall number
        "mov rsi, r12",  // arg1
        "mov rdx, r11",  // arg2
        "mov rcx, r10",  // arg3
        "call {syscall_entry}",

        // Return value is in rax

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

        // Return from syscall
        "sysretq",
        syscall_entry = sym syscall_entry
    );
}

/// Initialize syscall support
///
/// This function:
/// 1. Initializes the syscall table
/// 2. Adds the syscall entry to the IDT at vector 0x80
pub unsafe fn init_syscalls() -> Result<(), &'static str> {
    // Initialize syscall table
    init();

    // Get kernel code segment selector
    let kernel_cs: u16;
    core::arch::asm!(
        "mov {0:x}, cs",
        out(reg) kernel_cs,
        options(nomem, nostack, preserves_flags)
    );

    // Set up IDT entry for syscall (vector 0x80)
    let syscall_handler = syscall_stub as u64;

    // Use a trap gate for syscall (doesn't clear IF flag)
    // Type attributes: Present=0x80, DPL=3 (user level), Type=0xEF (trap gate)
    const SYSCALL_TYPE_ATTR: u8 = 0xEF; // Trap gate, user-level

    use crate::runtime::IdtEntry;
    let entry = IdtEntry::new(syscall_handler, kernel_cs, SYSCALL_TYPE_ATTR);

    // Add to IDT at vector 0x80
    use crate::runtime::IDT;
    IDT[0x80] = entry;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(SYS_WRITE, 1);
        assert_eq!(SYS_READ, 2);
        assert_eq!(SYS_EXIT, 60);
        assert_eq!(STDIN_FD, 0);
        assert_eq!(STDOUT_FD, 1);
        assert_eq!(STDERR_FD, 2);
    }

    #[test]
    fn test_max_syscalls() {
        assert_eq!(MAX_SYSCALLS, 128);
    }
}
