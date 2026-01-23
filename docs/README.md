# Rustux OS - UEFI Kernel

**Current Status:** Phase 6 COMPLETE - Interactive Shell Running

## Overview

Rustux is a hobby operating system written in Rust. It boots directly as a UEFI application (no GRUB, no Linux kernel) and provides an interactive shell with a Dracula-themed interface.

### Current State (January 2025)

- **Phase 6 Complete:** Interactive shell with PS/2 keyboard and framebuffer console
- **Direct UEFI Boot:** Standalone BOOTX64.EFI, no external bootloader
- **Multi-process:** Round-robin scheduler with process table
- **Syscalls:** read, write, open, close, lseek, spawn, exit, getpid, getppid, yield
- **Filesystem:** VFS abstraction with embedded ramdisk

### Boot Flow

```
UEFI Firmware → BOOTX64.EFI → Kernel → Init (PID 1) → Shell (PID 2)
```

---

## Core Features (Phase 6)

### 1. UEFI Boot

**Location:** `src/main.rs`, `src/arch/amd64/uefi.rs`

The kernel is a standalone UEFI application that:
- Calls `ExitBootServices()` to take full hardware control
- Sets up x86_64 page tables with proper isolation
- Configures IDT for interrupt handling
- Initializes scheduler and process management

**Status:** ✅ Complete

---

### 2. Process Management

**Location:** `src/process/mod.rs`, `src/process/table.rs`, `src/sched/round_robin.rs`

- **Process Table:** 256 slots, indexed by PID
- **Round-Robin Scheduler:** Time-sliced context switching
- **Context Switch:** Assembly switch.S for saving/restoring CPU state
- **Init Process:** PID 1 auto-spawns shell on boot

**Status:** ✅ Complete

---

### 3. Memory Management

**Location:** `src/mm/pmm.rs`, `src/arch/amd64/mmu.rs`

- **Physical Memory Manager:** Bitmap-based page allocation
- **Virtual Memory:** 4-level page tables (PML4 → PDPT → PD → PT)
- **Page Table Isolation:** Separate kernel/userspace page tables per process
- **Address Spaces:** Per-process virtual address spaces with 64MB heap

**Status:** ✅ Complete

---

### 4. Syscall Interface

**Location:** `src/syscall/mod.rs`, `src/arch/amd64/syscall.rs`

- **int 0x80 Interface:** Software interrupt for userspace→kernel transitions
- **Syscall Table:** Dispatches to kernel functions
- **Implemented Syscalls:**
  - `sys_read()` - Read from file descriptor
  - `sys_write()` - Write to file descriptor
  - `sys_open()` - Open file from ramdisk
  - `sys_close()` - Close file descriptor
  - `sys_lseek()` - Seek in file
  - `sys_spawn()` - Spawn new process from ELF
  - `sys_exit()` - Terminate process
  - `sys_getpid()` - Get process ID
  - `sys_getppid()` - Get parent process ID
  - `sys_yield()` - Yield CPU to scheduler

**Status:** ✅ Complete

---

### 5. ELF Loading

**Location:** `src/exec/elf.rs`, `src/exec/process_loader.rs`

- **ELF64 Parser:** Loads 64-bit ELF binaries
- **Segment Mapping:** Maps LOAD segments into process address space
- **Entry Point:** Jumps to ELF entry point in userspace
- **Dynamic Loading:** Static linking only (no dynamic loader yet)

**Status:** ✅ Complete

---

### 6. VFS + Ramdisk

**Location:** `src/fs/vfs.rs`, `src/fs/ramdisk.rs`

- **VFS Abstraction:** File operations (read, write, open, close, seek)
- **Ramdisk:** Embedded filesystem with ELF binaries
- **File Types:** Regular files (no directories yet)
- **Build Integration:** `build.rs` embeds userspace binaries

**Status:** ✅ Complete

---

### 7. PS/2 Keyboard Driver

**Location:** `src/drivers/keyboard/`

- **Hardware:** IRQ1, ports 0x60 (data) and 0x64 (command/status)
- **Scancode Set 1:** Converts scancodes to ASCII
- **Modifier Keys:** Shift, Ctrl, Alt, Caps Lock tracking
- **Circular Buffer:** Lock-free ring buffer for keyboard events
- **Blocking Read:** sys_read() blocks on stdin until character available

**Status:** ✅ Complete

---

### 8. Framebuffer Console

**Location:** `src/drivers/display/`

- **UEFI GOP:** Gets framebuffer from UEFI Graphics Output Protocol
- **PSF2 Fonts:** 8x16 pixel bitmap font
- **Text Console:** Character grid with scrolling
- **Colors:** RGB565 pixel format, Dracula theme colors
- **sys_write Routing:** stdout/stderr writes to framebuffer

**Status:** ✅ Complete

---

### 9. Interactive Shell

**Location:** `test-userspace/shell/shell.c`

- **C Implementation:** Statically linked, no stdlib
- **Built-in Commands:** help, clear, echo, ps, exit
- **Command Parser:** Tokenizes input and executes
- **Program Spawning:** Uses sys_spawn() to run ELF binaries
- **Dracula Theme:** ANSI color codes for styling

**Status:** ✅ Complete

---

## Architecture Support

### x86_64 (AMD64) - ✅ Primary Target

- UEFI boot
- 4-level page tables
- Interrupt handling (IDT, IRQ, exceptions)
- Context switching
- All drivers implemented

### ARM64 - ⏳ Planned

Cross-architecture design ready, hardware testing needed.

### RISC-V - ⏳ Planned

Cross-architecture design ready, hardware testing needed.

---

## What's NOT Implemented Yet

### Phase 7 - Minimal GUI (Planned)

- USB HID driver (keyboard + mouse)
- Framebuffer mapping into userspace
- GUI server process
- GUI client library

### Missing Features

- USB HID driver (PS/2 only currently)
- Networking stack
- Persistent filesystem (Ext2/3)
- Dynamic linking
- Multi-threading (single-threaded processes only)
- Signal handling
- Pipes and IPC
- Directories in VFS (flat filesystem only)

---

## Build Instructions

### Prerequisites

```bash
# Rust toolchain (UEFI target)
rustup target add x86_64-unknown-uefi

# GCC for cross-compiling userspace C programs
apt install gcc-x86-64-linux-gnu

# Image creation tools
apt install parted dosfstools coreutils
```

### Build

```bash
cd /var/www/rustux.com/prod/rustux

# Build kernel (UEFI application)
cargo build --release --target x86_64-unknown-uefi

# Build userspace C programs
cd test-userspace
x86_64-linux-gnu-gcc -static -nostdlib -fno-stack-protector \
    shell.c -o shell.elf
x86_64-linux-gnu-gcc -static -nostdlib -fno-stack-protector \
    init.c -o init.elf

# Build live USB image
./build-live-image.sh
```

---

## System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| **Architecture** | x86_64 (AMD64) | x86_64 (AMD64) |
| **Boot** | UEFI 2.0 | UEFI 2.3+ |
| **RAM** | 512 MB | 1 GB |
| **Storage** | 128 MB (USB) | 4 GB |
| **Input** | PS/2 Keyboard | PS/2 or USB HID* |

\* USB HID support planned for Phase 7

---

## Dracula Theme (MANDATORY INVARIANT)

The Dracula color palette is the default system theme and must survive:

- Kernel rebuilds
- CLI refactors
- Framebuffer rewrites
- GUI introduction later

**Canonical Dracula Colors:**
```
FG_DEFAULT = #F8F8F2  (r: 248, g: 248, b: 242)
BG_DEFAULT = #282A36  (r: 40, g: 42, b: 54)
CYAN       = #8BE9FD  (r: 139, g: 233, b: 253)
PURPLE     = #BD93F9  (r: 189, g: 147, b: 249)
GREEN      = #50FA7B  (r: 80, g: 250, b: 123)
RED        = #FF5555  (r: 255, g: 85, b: 85)
ORANGE     = #FFB86C  (r: 255, g: 184, b: 108)
YELLOW     = #F1FA8C  (r: 241, g: 250, b: 140)
```

---

## Development Roadmap

### Phase 4: Userspace & Process Execution ✅ COMPLETE
- ELF loading with segment mapping
- Per-process address spaces
- Page table isolation
- int 0x80 syscall interface
- First userspace instruction execution

### Phase 5: Process Management & Essential Syscalls ✅ COMPLETE
- Process table with 256 slots
- Round-robin scheduler with context switching
- Ramdisk for embedded files
- sys_spawn() for spawning from paths
- Init process (PID 1) auto-loads on boot

### Phase 6: Input, Display, Interactive Shell ✅ COMPLETE
- PS/2 keyboard driver (IRQ1, ports 0x60/0x64)
- Scancode to ASCII conversion with modifier tracking
- Framebuffer driver with PSF2 fonts
- Text console with scrolling
- Interactive C shell with Dracula theme

### Phase 7: Minimal GUI 🚧 PLANNED
- USB HID driver (keyboard + mouse)
- Framebuffer mapping syscall
- GUI server process (rustica-gui)
- GUI client library (librustica_gui)

---

## Documentation

- **[BUILD.md](docs/BUILD.md)** - Live USB build instructions
- **[IMAGE.md](docs/IMAGE.md)** - System architecture and boot flow
- **[PLAN.md](docs/PLAN.md)** - Development roadmap with detailed phase specs

---

## Project Structure

```
/var/www/rustux.com/prod/rustux/
├── src/
│   ├── arch/amd64/       # Architecture-specific code
│   ├── drivers/          # Device drivers (keyboard, display)
│   ├── exec/             # ELF loading, process creation
│   ├── fs/               # VFS, ramdisk
│   ├── init.rs           # Kernel initialization
│   ├── lib.rs            # Library entry point
│   ├── main.rs           # UEFI entry point
│   ├── mm/               # Physical memory manager
│   ├── process/          # Process table, context switching
│   ├── sched/            # Round-robin scheduler
│   └── syscall/          # System call handlers
├── test-userspace/       # C programs (shell, init, hello, counter)
│   ├── init.c            # First userspace process (PID 1)
│   └── shell/
│       └── shell.c       # Interactive shell
├── build.rs              # Embed ramdisk with userspace binaries
├── build-live-image.sh   # Live USB build script
└── PLAN.md               # Development roadmap
```

---

## Contributing

See PLAN.md for:
- Coding standards
- Development workflow
- Phase specifications
- Technical decisions

---

## License

MIT License - See LICENSE file for details.

---

## Links

- **Repository:** https://github.com/gitrustux/rustux
- **Website:** https://rustux.com
- **Issue Tracker:** https://github.com/gitrustux/rustux/issues

---

**Last Updated:** January 23, 2025
**Status:** Phase 6 COMPLETE - Interactive shell running with Dracula theme
