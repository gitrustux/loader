# Rustux EFI Loader

**IMPORTANT: This is a TRANSITION KERNEL for live boot validation.**

This directory contains a monolithic UEFI kernel (`kernel-efi/`) used to validate Phase 6 features before integration into the canonical microkernel.

## Purpose

The `kernel-efi/` transition kernel serves as:
- **Live boot validation** - Single UEFI application for USB testing
- **Hardware testing** - PS/2 keyboard, framebuffer, UEFI GOP
- **Feature validation** - Process management, syscalls, shell interaction

**This is NOT the canonical Rustux kernel.** The real kernel is in `/var/www/rustux.com/prod/rustux/` (microkernel architecture).

## Migration Plan

Phase 6D will migrate validated subsystems from this transition kernel into the canonical microkernel:
- PS/2 keyboard driver → `rustux/src/drivers/keyboard/`
- Framebuffer console → `rustux/src/drivers/display/`
- Live boot tooling → `rustux/src/boot/uefi/`

The monolithic `kernel-efi/` will be retired after migration is complete.

## Directory Structure

```
loader/
├── kernel-efi/           # Monolithic UEFI transition kernel
│   ├── src/              # Kernel source (single-binary architecture)
│   ├── build.rs          # Embeds ramdisk with userspace binaries
│   └── target/           # Built kernel.efi
├── uefi-loader/          # UEFI bootloader (loads kernel.efi)
├── build-live-image.sh   # Live USB image creation script
├── README.md             # This file
└── .gitignore            # Git ignore patterns
```

## Building the Transition Kernel

```bash
cd /var/www/rustux.com/prod/loader/kernel-efi

# Build UEFI kernel
cargo build --release --target x86_64-unknown-uefi

# Build userspace programs
cd /var/www/rustux.com/prod/rustica/test-userspace
x86_64-linux-gnu-gcc -static -nostdlib -fno-stack-protector shell.c -o shell.elf
x86_64-linux-gnu-gcc -static -nostdlib -fno-stack-protector init.c -o init.elf
```

## Creating Live USB Image

```bash
cd /var/www/rustux.com/prod/loader

# Make script executable
chmod +x build-live-image.sh

# Build image
./build-live-image.sh

# Or with custom version
RUSTUX_VERSION=1.0.0 ./build-live-image.sh
```

**Output:** `/var/www/rustux.com/html/rustica/rustica-live-amd64-{VERSION}.img`

## Writing to USB

```bash
# Identify USB device
lsblk

# Write image (replace /dev/sdX)
sudo dd if=rustica-live-amd64-0.1.0.img of=/dev/sdX bs=4M status=progress conv=fsync
sudo sync
```

## Booting

1. Insert USB and restart computer
2. Enter boot menu (F12, F2, F10, Del, or Esc)
3. Select USB drive (look for "UEFI: USB...")
4. System boots to Rustux shell

## Validated Features (Phase 6)

| Feature | Status | Notes |
|---------|--------|-------|
| UEFI Direct Boot | ✅ | No GRUB, standalone BOOTX64.EFI |
| PS/2 Keyboard | ✅ | IRQ1, scancode set 1, modifiers |
| Framebuffer Console | ✅ | RGB565, PSF2 font (8x16), scrolling |
| Process Management | ✅ | 256-slot table, round-robin scheduler |
| Syscall Interface | ✅ | read, write, spawn, exit, getpid, yield |
| VFS + Ramdisk | ✅ | Embedded ELF binaries (init, shell, hello) |
| Interactive Shell | ✅ | C shell with Dracula theme |

## Dracula Theme (MANDATORY INVARIANT)

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

## System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| Architecture | x86_64 (AMD64) | x86_64 (AMD64) |
| Boot | UEFI 2.0 | UEFI 2.3+ |
| RAM | 512 MB | 1 GB |
| Storage | 128 MB (USB) | 4 GB |
| Input | PS/2 Keyboard | PS/2 or USB HID* |

\* USB HID support planned for Phase 7

## Repository

- **Git:** https://github.com/gitrustux/rustux-efi
- **Main Project:** https://github.com/gitrustux/rustux
- **Website:** https://rustux.com

## License

MIT License - See LICENSE file in root directory.

---

**Last Updated:** January 23, 2025
**Status:** Phase 6 COMPLETE - Transition kernel validated, awaiting microkernel migration
