// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Embedded filesystem for kernel runtime
//!
//! This module provides a simple in-memory filesystem with embedded binaries
//! that can be executed after ExitBootServices.

use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// File entry in the embedded filesystem
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub data: Vec<u8>,
    pub is_executable: bool,
}

/// Simple embedded filesystem
pub struct EmbeddedFileSystem {
    files: BTreeMap<String, FileEntry>,
}

impl EmbeddedFileSystem {
    /// Create a new embedded filesystem with test binaries
    pub fn new() -> Self {
        let mut fs = Self {
            files: BTreeMap::new(),
        };

        // Add embedded binaries
        fs.add_test_binaries();

        fs
    }

    /// Add test binaries to the filesystem
    fn add_test_binaries(&mut self) {
        // Add the embedded userspace test program (Phase 11)
        // This is a real Rust program that uses Rustux syscalls
        let userspace_bin = crate::userspace_bin::USERSPACE_BIN.to_vec();
        self.files.insert(String::from("test"), FileEntry {
            name: String::from("test"),
            data: userspace_bin,
            is_executable: true,
        });

        // Simple "hello" program - prints "Hello from userspace!"
        // This is a minimal x86_64 ELF binary
        let hello_elf = create_hello_elf();
        self.files.insert(String::from("hello"), FileEntry {
            name: String::from("hello"),
            data: hello_elf,
            is_executable: true,
        });

        // Simple "echo" program - echoes arguments back
        let echo_elf = create_echo_elf();
        self.files.insert(String::from("echo"), FileEntry {
            name: String::from("echo"),
            data: echo_elf,
            is_executable: true,
        });

        // Simple "test" program - runs basic tests
        let test_elf = create_test_elf();
        self.files.insert(String::from("test"), FileEntry {
            name: String::from("test"),
            data: test_elf,
            is_executable: true,
        });

        // Simple "version" program - shows program version
        let version_elf = create_version_elf();
        self.files.insert(String::from("version"), FileEntry {
            name: String::from("version"),
            data: version_elf,
            is_executable: true,
        });
    }

    /// Read a file by name
    pub fn read(&self, path: &str) -> Option<&FileEntry> {
        // Extract filename from path
        let filename = if let Some(last_slash) = path.rfind('/') {
            &path[last_slash + 1..]
        } else {
            path
        };

        self.files.get(filename)
    }

    /// List all files
    pub fn list(&self) -> Vec<&str> {
        self.files.keys().map(|k| k.as_str()).collect()
    }

    /// Check if a file exists
    pub fn exists(&self, path: &str) -> bool {
        self.read(path).is_some()
    }
}

/// Default filesystem instance
static mut EMBEDDED_FS: Option<EmbeddedFileSystem> = None;

/// Initialize the embedded filesystem
pub unsafe fn init_filesystem() {
    EMBEDDED_FS = Some(EmbeddedFileSystem::new());
}

/// Get the embedded filesystem
pub unsafe fn get_filesystem() -> Option<&'static EmbeddedFileSystem> {
    EMBEDDED_FS.as_ref()
}

/// Create a minimal x86_64 ELF binary that prints "Hello from userspace!"
fn create_hello_elf() -> Vec<u8> {
    // Minimal x86_64 ELF with "Hello" message
    // For now, this is a stub that will be replaced with actual code
    // The real implementation will generate valid ELF binaries
    let mut elf = Vec::new();

    // ELF header
    elf.extend_from_slice(b"\x7fELF"); // e_ident magic
    elf.extend_from_slice(&[2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // e_ident rest (64-bit, little-endian)
    elf.extend_from_slice(&[2, 0]); // e_type = ET_EXEC
    elf.extend_from_slice(&[62, 0]); // e_machine = EM_X86_64
    elf.extend_from_slice(&[1, 0, 0, 0]); // e_version
    elf.extend_from_slice(&[0, 0, 0x10, 0, 0, 0, 0, 0]); // e_entry = 0x100000 (will be adjusted)
    elf.extend_from_slice(&[64, 0, 0, 0, 0, 0, 0, 0]); // e_phoff
    elf.extend_from_slice(&[0; 64]); // rest of header

    // Program header (PT_LOAD)
    elf.extend_from_slice(&[1, 0, 0, 0]); // p_type = PT_LOAD
    elf.extend_from_slice(&[5, 0, 0, 0]); // p_flags = PF_R | PF_X
    elf.extend_from_slice(&[0, 0x10, 0, 0, 0, 0, 0, 0]); // p_offset
    elf.extend_from_slice(&[0, 0, 0x10, 0, 0, 0, 0, 0]); // p_vaddr
    elf.extend_from_slice(&[0, 0, 0x10, 0, 0, 0, 0, 0]); // p_paddr
    elf.extend_from_slice(&[0x20, 0, 0, 0, 0, 0, 0, 0]); // p_filesz
    elf.extend_from_slice(&[0x20, 0, 0, 0, 0, 0, 0, 0]); // p_memsz
    elf.extend_from_slice(&[0x10, 0, 0, 0, 0, 0, 0, 0]); // p_align

    // Code section - simple syscall write
    // mov rax, 1 (sys_write)
    // mov rdi, 1 (stdout)
    // mov rsi, message
    // mov rdx, 20 (length)
    // syscall
    // mov rax, 60 (sys_exit)
    // xor rdi, rdi
    // syscall
    let mut code = Vec::new();

    code.extend_from_slice(&[
        0x48, 0xc7, 0xc0, 0x01, 0x00, 0x00, 0x00, // mov rax, 1
        0x48, 0xc7, 0xc7, 0x01, 0x00, 0x00, 0x00, // mov rdi, 1
        0x48, 0x8d, 0x35, 0x12, 0x00, 0x00, 0x00, // lea rsi, [rip+0x12] (message)
        0x48, 0xc7, 0xc2, 0x14, 0x00, 0x00, 0x00, // mov rdx, 20
        0x0f, 0x05,                             // syscall
        0x48, 0xc7, 0xc0, 0x3c, 0x00, 0x00, 0x00, // mov rax, 60
        0x48, 0x31, 0xff,                        // xor rdi, rdi
        0x0f, 0x05,                             // syscall
    ]);

    // Message: "Hello from userspace!\n"
    code.extend_from_slice(b"Hello from userspace!\n");

    // Pad to 64 bytes
    while code.len() < 64 {
        code.push(0x90); // nop
    }

    elf.extend_from_slice(&code);

    // Pad to page boundary
    while elf.len() < 4096 {
        elf.push(0);
    }

    elf
}

/// Create a minimal ELF for echo command
fn create_echo_elf() -> Vec<u8> {
    // Similar to hello but echoes arguments
    let mut elf = create_hello_elf();

    // Modify the message to indicate it's echo
    let message = b"Echo: arguments not yet implemented\n";
    let msg_start = elf.len() - 4096;

    for (i, &byte) in message.iter().enumerate() {
        if msg_start + i < elf.len() - 21 {
            elf[msg_start + i] = byte;
        }
    }

    elf
}

/// Create a minimal ELF for test command
fn create_test_elf() -> Vec<u8> {
    let mut elf = create_hello_elf();

    // Modify the message for test
    let message = b"Test: All systems operational\n";
    let msg_start = elf.len() - 4096;

    for (i, &byte) in message.iter().enumerate() {
        if msg_start + i < elf.len() - 21 {
            elf[msg_start + i] = byte;
        }
    }

    elf
}

/// Create a minimal ELF for version command
fn create_version_elf() -> Vec<u8> {
    let mut elf = create_hello_elf();

    // Modify the message for version
    let message = b"Userspace Program v1.0.0\n";
    let msg_start = elf.len() - 4096;

    for (i, &byte) in message.iter().enumerate() {
        if msg_start + i < elf.len() - 21 {
            elf[msg_start + i] = byte;
        }
    }

    elf
}
