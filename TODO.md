# Rustux Kernel - Development Roadmap

## Status: STABLE POST-UEFI CHECKPOINT ✅

**Checkpoint Date**: 2025-01-13

**Current State**: The kernel successfully exits UEFI boot services, transitions to runtime mode, and all core subsystems are operational. This is a stable baseline for future development.

---

## Completed Phases ✅

### Phase 1: Clean Up Frozen Zone Code ✅
- [x] Implement frozen zone between GetMemoryMap and ExitBootServices
- [x] Remove ALL debug output from frozen zone
- [x] Ensure NO allocations, prints, or protocol calls in frozen zone
- [x] Verify ExitBootServices succeeds (QEMU keeps running)
- [x] Confirm UEFI console stops working after exit (expected)

**Commit**: `7a3c86a` - Implement kernel allocator from memory map (Phase 4)

---

### Phase 2: Remove Probe Loop & Add Heartbeat ✅
- [x] Remove infinite probe loop after ExitBootServices
- [x] Add visual heartbeat to confirm forward progress
- [x] Implement VGA text buffer output at 0xB8000
- [x] Add animated spinner for visual confirmation

**Commit**: `7a3c86a` - Implement kernel allocator from memory map (Phase 4)

---

### Phase 3: Native Console Driver (Partial) ✅
- [x] Write directly to VGA text buffer at 0xB8000
- [x] Implement status message display
- [x] Add color support for different components
- [ ] Implement text scrolling (PENDING)
- [ ] Implement full console driver with cursor (PENDING)

**VGA Status Output** (Row 0):
- Columns 0-10: "RUNTIME MODE" (Blue on white)
- Column 80-90: "ALLOC OK!" (Yellow on black)
- Column 90-100: "IDT OK!" (Green on black)
- Column 100-110: "APIC OK!" (Magenta on black)
- Column 110-120: "SCHED OK!" (Red on black)
- Column 120-135: "UEFI DISABLED!" (Blue on black)
- Column 0 (animated): Spinning spinner `/-\|`

---

### Phase 4: Memory Management ✅
- [x] Create kernel-owned bump allocator from memory map
- [x] Mark boot services memory as available for reuse
- [x] Parse UEFI memory map for usable regions
- [x] Initialize bump allocator with 1MB heap
- [ ] Implement heap allocation with bump → slab → buddy progression (PENDING)
- [ ] Add memory page management (PENDING)
- [ ] Implement virtual memory setup (PENDING)
- [ ] Add memory protection (PENDING)

**Commit**: `7a3c86a` - Implement kernel allocator from memory map (Phase 4)

---

### Phase 5: Runtime Initialization Order ✅
- [x] Implement memory allocator init (MUST BE FIRST)
- [x] Implement x86_64 IDT for exception handlers
- [x] Add interrupt controller init (Local APIC)
- [x] Add scheduler stub
- [ ] Add native console init (PENDING - Phase 8)

**Commits**:
- `b75e900` - Implement x86_64 IDT for exception handlers (Phase 5)
- `ee91dfa` - Implement APIC interrupt controller and scheduler stub (Phase 5)

---

### Phase 6: Disable UEFI Services Permanently ✅
- [x] Zero out UEFI system table pointer
- [x] Disable global UEFI allocator
- [x] Add runtime mode flag checked by all UEFI code
- [x] Add safety wrapper around boot services calls
- [x] Implement `assert_runtime_mode()` safety function
- [x] Document "Runtime Mode Only" API boundary

**Commit**: `b457858` - Disable UEFI services permanently (Phase 6)

---

### Phase 7: External Program Execution (Loader Only) ✅
- [x] Implement ELF loader (parse PT_LOAD headers)
- [x] Load ELF into kernel memory with proper alignment
- [x] Add embedded filesystem with test binaries
- [x] Test ELF loading and display status on VGA
- [ ] Set up process memory map (PENDING)
- [ ] Implement process switching (PENDING)
- [ ] Add system call interface (PENDING - Phase 10)

**Commit**: `e2d8ad7` - Implement ELF loader test (Phase 7)

**Note**: ELF binaries use Linux syscalls which don't work in this kernel. The loader is demonstrated but binaries cannot be executed without kernel-specific system call interface.

---

## Known Limitations

### 1. No Interactive Input in Runtime Mode
**Issue**: After ExitBootServices, there's no way to receive keyboard input.

**Impact**: Cannot run interactive programs or shell.

**Workaround**: None currently. Need keyboard driver (Phase 9).

---

### 2. ELF Binaries Use Linux Syscalls
**Issue**: Embedded ELF binaries use `sys_write` and `sys_exit` which are Linux-specific.

**Impact**: Binaries cannot execute even though they load successfully.

**Workaround**: Need to implement Rustux syscall ABI (Phase 10).

---

### 3. Serial Port I/O Hangs
**Issue**: Writing to COM1 (0x3F8) causes QEMU/OVMF to hang.

**Root Cause**: OVMF emulates serial lazily; requires proper initialization sequence.

**Workaround**: Use VGA text mode for output. Serial is reserved for debugging only.

**Status**: Not blocking progress. Can be investigated later.

---

### 4. No Process Isolation
**Issue**: All code runs in kernel mode with no memory protection.

**Impact**: No security boundary between kernel and userspace.

**Workaround**: Acceptable for current development stage. Will be addressed in future phases.

---

### 5. Bump Allocator Only
**Issue**: Simple bump allocator with no deallocation or memory reuse.

**Impact**: Memory is not reclaimed, limiting long-running processes.

**Workaround**: Acceptable for current stage. Upgrade to slab/buddy allocator later.

---

## Next Objective: Minimal Interactive Runtime Environment

**Goal**: "I type text → I see text" in runtime mode.

**Priority**: Stability > Elegance. Do NOT refactor existing kernel subsystems unless required.

---

## Phase 8: Native Console Driver (HIGHEST PRIORITY)

### Requirements
- Implement VGA text-mode console (0xB8000)
- Fixed 80x25 dimensions
- White/yellow text on black background
- Scrolling support
- NO heap allocation
- Must work after ExitBootServices

### Tasks
1. **Scrolling Implementation**
   - [ ] Implement scroll_up() function
   - [ ] Move all rows up by 1 when cursor reaches bottom
   - [ ] Clear bottom row after scroll

2. **Cursor Management**
   - [ ] Track cursor position (row, column)
   - [ ] Implement cursor wrapping (row end → next row)
   - [ ] Auto-scroll when cursor exceeds last row

3. **Character Output**
   - [ ] Implement putc() for single character
   - [ ] Implement puts() for string output
   - [ ] Handle newline (\n) and carriage return (\r)

4. **Color Support**
   - [ ] Default color: White on black (0x0F00)
   - [ ] Support for custom colors if needed

### Constraints
- NO heap allocation (no Vec, String, Box)
- Use only stack-allocated buffers
- Direct memory writes to 0xB8000

---

## Phase 9: Native Keyboard Driver

### Requirements
- Implement PS/2 keyboard driver (IRQ1)
- Handle scan codes directly
- ASCII mapping for basic keys only

### Supported Keys
- [ ] Letters (a-z, A-Z)
- [ ] Numbers (0-9)
- [ ] Space
- [ ] Backspace
- [ ] Enter
- [ ] Comma (,), Dash (-), Period (.)

### Ignored Keys (For Now)
- Function keys (F1-F12)
- Arrow keys
- Control, Alt, Shift
- Special keys

### Implementation
1. **IRQ1 Handler**
   - [ ] Add IRQ1 handler to IDT
   - [ ] Read scan code from port 0x60
   - [ ] Handle release codes (0x80 prefix)

2. **Scan Code to ASCII**
   - [ ] Implement lookup table for basic keys
   - [ ] Handle shift modifier (if needed)
   - [ ] Buffer input in circular buffer

3. **Input Buffer**
   - [ ] Fixed-size buffer (no heap allocation)
   - [ ] Support for buffered read

### Constraints
- Use in/out instructions for port I/O
- NO heap allocation
- Minimal interrupt handling (EOI required)

---

## Phase 10: Minimal Syscall Interface

### Requirements
- Implement ONLY: sys_write(fd=1), sys_read(fd=0), sys_exit
- Define Rustux syscall ABI (NOT Linux)
- Trap via syscall instruction

### Syscall ABI (Rustux)
```
syscall number: rax
arguments: rdi, rsi, rdx
return value: rax
```

### Syscall Numbers
- sys_write: 1
- sys_read: 2
- sys_exit: 60

### Implementation
1. **Syscall Entry Point**
   - [ ] Add syscall handler to IDT (vector 0x80)
   - [ ] Save/restore registers
   - [ ] Dispatch based on rax

2. **sys_write(fd=1)**
   - [ ] Write buffer to console
   - [ ] rdi: fd (must be 1 for stdout)
   - [ ] rsi: buffer pointer
   - [ ] rdx: length
   - [ ] Return: bytes written

3. **sys_read(fd=0)**
   - [ ] Read from keyboard buffer
   - [ ] rdi: fd (must be 0 for stdin)
   - [ ] rsi: buffer pointer
   - [ ] rdx: length
   - [ ] Return: bytes read

4. **sys_exit**
   - [ ] Terminate current process
   - [ ] rdi: exit code
   - [ ] Return: does not return

### Constraints
- Minimal implementation only
- No fd translation (fd must be 0 or 1)
- NO Linux compatibility layer

---

## Phase 11: Minimal Userspace Test Program

### Requirements
- Write a tiny Rust userspace binary
- Prints "hello"
- Reads input
- Echoes input back
- No libc
- No Linux syscalls

### Implementation
1. **Custom Runtime**
   - [ ] Implement _start function
   - [ ] Handle syscall instruction
   - [ ] No standard library

2. **Test Program**
   ```rust
   #![no_std]
   #![no_main]

   use core::arch::asm;

   #[no_mangle]
   pub extern "C" fn _start() -> ! {
       // Print "hello\n"
       let msg = b"hello\n";
       syscall_write(1, msg.as_ptr(), msg.len());

       // Read input
       let mut buffer = [0u8; 64];
       let n = syscall_read(0, buffer.as_mut_ptr(), buffer.len());

       // Echo back
       syscall_write(1, buffer.as_ptr(), n);

       // Exit
       syscall_exit(0);
   }

   fn syscall_write(fd: u64, buf: *const u8, len: u64) {
       unsafe {
           asm!(
               "syscall",
               in("rax") 1u64,
               in("rdi") fd,
               in("rsi") buf,
               in("rdx") len
           );
       }
   }

   fn syscall_read(fd: u64, buf: *mut u8, len: u64) -> u64 {
       let mut ret = 0;
       unsafe {
           asm!(
               "syscall",
               inlateout("rax") ret => _,
               in("rdi") fd,
               in("rsi") buf,
               in("rdx") len
           );
       }
       ret
   }

   fn syscall_exit(code: u64) -> ! {
       unsafe {
           asm!(
               "syscall",
               in("rax") 60u64,
               in("rdi") code
           );
       }
       loop {}
   }
   ```

### Build
- [ ] Create separate binary crate
- [ ] Build as freestanding binary
- [ ] Embed in kernel filesystem

---

## Phase 12: Shell Stub (NOT Full CLI)

### Requirements
- Single loop: read line, print it back
- NO command execution yet
- Simple echo interface

### Implementation
1. **Main Loop**
   - [ ] Print prompt "rustux> "
   - [ ] Read line using sys_read
   - [ ] Print line using sys_write
   - [ ] Repeat

2. **Line Editing**
   - [ ] Handle backspace
   - [ ] Handle enter
   - [ ] No advanced editing (no arrow keys, no history)

### Constraints
- Minimal implementation only
- NO command parsing
- NO built-in commands
- NO piping or redirection

---

## What NOT To Work On (Yet)

### Package Management
- ❌ Do NOT implement package manager
- ❌ Do NOT add package format
- ❌ Do NOT add dependency resolution

### ELF Compatibility
- ❌ Do NOT add Linux ELF compatibility layer
- ❌ Do NOT support dynamic linking
- ❌ Do NOT add ELF interpreter

### Networking
- ❌ Do NOT add network stack
- ❌ Do NOT implement TCP/IP
- ❌ Do NOT add socket interface

### GUI
- ❌ Do NOT add windowing system
- ❌ Do NOT implement graphics beyond text console
- ❌ Do NOT add compositor

### Scheduler Improvements
- ❌ Do NOT implement scheduling algorithms
- ❌ Do NOT add priority scheduling
- ❌ Do NOT implement preemption

### Memory Allocator Improvements
- ❌ Do NOT upgrade to slab/buddy allocator yet
- ❌ Do NOT add memory defragmentation
- ❌ Do NOT implement virtual memory

---

## Testing & Debugging

### Current Testing Setup
```bash
# Build
cd /var/www/rustux.com/prod/kernel/kernel-efi
cargo build --release --target x86_64-unknown-uefi

# Update disk image
LOOPDEV=$(losetup -f)
losetup -P $LOOPDEV /var/www/rustux.com/html/rustica/rustica-live-amd64-0.1.0.img
mount ${LOOPDEV}p1 /mnt/rustica-test
cp target/x86_64-unknown-uefi/release/rustux-kernel-efi.efi /mnt/rustica-test/EFI/Rustux/kernel.efi
umount /mnt/rustica-test
losetup -d $LOOPDEV

# Test
timeout 5 qemu-system-x86_64 -m 512M \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive format=raw,file=/var/www/rustux.com/html/rustica/rustica-live-amd64-0.1.0.img \
  -display none -serial stdio
```

### Success Indicators (Post-ExitBootServices)
- ✅ QEMU keeps running (ExitBootServices succeeded)
- ✅ VGA shows "RUNTIME MODE" with status messages
- ✅ VGA shows animated spinner
- ✅ VGA shows "ELF LOADED! ENTRY=0x..." on second row
- ✅ No "FAILED" or "HALTING" messages

### VGA Layout (Current)
```
Row 0: [RUNTIME MODE][ALLOC OK!][IDT OK!][APIC OK!][SCHED OK!][UEFI DISABLED!]
Row 1: [ELF LOADED! ENTRY=0x..............]  OR  [ELF LOAD ERR]
```

---

## Cross-Architecture Status

### x86_64 (kernel-efi) ✅
- **Status**: Stable checkpoint
- **Boot**: UEFI
- **ExitBootServices**: ✅ Working
- **Runtime Mode**: ✅ Fully operational
- **Allocator**: ✅ Bump allocator
- **IDT**: ✅ 32 exception vectors
- **APIC**: ✅ Enabled and configured
- **Scheduler**: ✅ Stub initialized
- **UEFI Disable**: ✅ Permanently disabled
- **ELF Loader**: ✅ Loading works, execution pending syscalls

### ARM64 ✅
- **Status**: Custom bootloader (not UEFI)
- **Boot**: Custom bootloader
- **Progress**: On par with x86_64
- **Location**: `/var/www/rustux.com/prod/kernel/src/arch/arm64`

### RISC-V ✅
- **Status**: Custom bootloader (not UEFI)
- **Boot**: Custom bootloader
- **Progress**: On par with x86_64
- **Location**: `/var/www/rustux.com/prod/kernel/src/arch/riscv64`

---

## Repository Status

### Primary Repository
- **URL**: `git@github.com:gitrustux/kernel.git`
- **Branch**: `main`
- **Latest Commit**: `e2d8ad7` - Implement ELF loader test (Phase 7)

### Repository Structure
```
/var/www/rustux.com/prod/kernel/
├── kernel-efi/          # x86_64 UEFI kernel (this work)
│   └── src/
│       ├── main.rs      # UEFI entry point
│       ├── runtime.rs   # Runtime subsystems
│       ├── console.rs   # Console utilities
│       ├── filesystem.rs # Embedded filesystem
│       ├── native_console.rs
│       └── theme.rs
├── src/                 # Main kernel (all arches)
│   └── arch/
│       ├── arm64/       # ARM64 support
│       └── riscv64/     # RISC-V support
├── rustux_macros/       # Procedural macros
└── Cargo.toml
```

---

## Git Commit History (Recent)

```
e2d8ad7 - Implement ELF loader test (Phase 7)
b457858 - Disable UEFI services permanently (Phase 6)
ee91dfa - Implement APIC interrupt controller and scheduler stub (Phase 5)
b75e900 - Implement x86_64 IDT for exception handlers (Phase 5)
7a3c86a - Implement kernel allocator from memory map (Phase 4)
```

---

## References

- UEFI Specification: ExitBootServices() requirements
- x86_64 ABI: System V AMD64 ABI
- PS/2 Keyboard: Scan code sets
- VGA Text Mode: Memory layout and color codes
- ELF Format: 64-bit ELF specification

---

**Last Updated**: 2025-01-13
**Checkpoint**: Stable Post-UEFI Kernel - All Core Subsystems Operational
**Next Objective**: Minimal Interactive Runtime Environment (Phases 8-12)
