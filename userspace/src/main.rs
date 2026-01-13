#![no_std]
#![no_main]

//! Minimal userspace test program for Rustux
//!
//! This program:
//! 1. Prints "hello" to stdout
//! 2. Reads input from stdin
//! 3. Echoes the input back
//! 4. Exits with code 0
//!
//! It uses the Rustux syscall ABI (NOT Linux).

use core::arch::asm;

/// Rustux syscall numbers
const SYS_WRITE: u64 = 1;
const SYS_READ: u64 = 2;
const SYS_EXIT: u64 = 60;

/// File descriptors
const STDIN: u64 = 0;
const STDOUT: u64 = 1;

/// Syscall write wrapper
#[inline(always)]
unsafe fn syscall_write(fd: u64, buf: *const u8, len: u64) {
    asm!(
        "syscall",
        in("rax") SYS_WRITE,
        in("rdi") fd,
        in("rsi") buf,
        in("rdx") len,
        clobber_abi("system")
    );
}

/// Syscall read wrapper
#[inline(always)]
unsafe fn syscall_read(fd: u64, buf: *mut u8, len: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        inlateout("rax") SYS_READ => ret,
        in("rdi") fd,
        in("rsi") buf,
        in("rdx") len,
        lateout("rcx") _,
        lateout("r11") _,
        clobber_abi("system")
    );
    ret
}

/// Syscall exit wrapper
#[inline(always)]
unsafe fn syscall_exit(code: u64) -> ! {
    asm!(
        "syscall",
        in("rax") SYS_EXIT,
        in("rdi") code,
        clobber_abi("system")
    );
    loop {}
}

/// Entry point for the userspace program
#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        // 1. Print "hello\n" to stdout
        let msg = b"hello\n";
        syscall_write(STDOUT, msg.as_ptr(), msg.len() as u64);

        // 2. Read input from stdin
        let mut buffer = [0u8; 64];
        let n = syscall_read(STDIN, buffer.as_mut_ptr(), buffer.len() as u64);

        // 3. Echo input back to stdout
        if n > 0 {
            // Print prompt
            let echo_msg = b"You typed: ";
            syscall_write(STDOUT, echo_msg.as_ptr(), echo_msg.len() as u64);

            // Print the input
            syscall_write(STDOUT, buffer.as_ptr(), n);

            // Print newline
            let newline = b"\n";
            syscall_write(STDOUT, newline.as_ptr(), newline.len() as u64);
        }

        // 4. Exit with code 0
        syscall_exit(0);
    }
}

/// Panic handler
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        syscall_exit(1);
    }
}
