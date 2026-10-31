# Rustux OS - UEFI Kernel with USB HID Keyboard

> **Legacy kernel.** This describes `kernel-efi`, the single-binary UEFI kernel used for the live USB image until the Rustux kernel (`gitrustux/rustux`) can replace it. It was previously kept as `rustica/docs/README.md`. The latest USB/xHCI work is on branch `wip/xhci-2026-08`; see the Rustux kernel's [`docs/usb-keyboard.md`](https://github.com/gitrustux/rustux/blob/main/docs/usb-keyboard.md) for status.

**Status:** 🟡 Phase 7A IN PROGRESS | USB HID Keyboard with xHCI/EHCI Transfers (Polling-based)

---

## Project Overview

**Rustux OS** is a hobby operating system written in Rust, featuring a native UEFI microkernel with USB HID keyboard support and a Dracula-themed interactive shell.

**Boot Flow:**
```
UEFI Firmware → BOOTX64.EFI → USB Init (xHCI/EHCI) → USB HID Keyboard → Shell (CLI)
```

---

## Current Status (January 26, 2026)

### ✅ Completed: Phase 7A - USB HID Keyboard Support

| Component | Status | Notes |
|-----------|--------|-------|
| **xHCI Controller** | ✅ Complete | PCI scan, controller init, port reset |
| **EHCI Controller** | ✅ Complete | USB 2.0 support for older laptops |
| **USB Controller Detection** | ✅ Complete | xHCI > EHCI priority, graceful fallback |
| **TRB Structures** | ✅ Complete | NormalTrb, SetupTrb, StatusTrb, EventTrb, LinkTrb |
| **Transfer Rings** | ✅ Complete | 16-entry rings with cycle bit management |
| **Event Rings** | ✅ Complete | 16-entry event ring with polling |
| **USB Device Enumeration** | ✅ Complete | Port scanning, keyboard detection |
| **Interrupt Transfers** | ✅ Complete | xHCI/EHCI polling implementation for HID data |
| **HID Report Parsing** | ✅ Complete | 256-entry keycode table, shift handling |
| **Keyboard Backend** | ✅ Complete | USB → PS/2 fallback, honest detection |
| **No-Keyboard Warning Fix** | ✅ Complete | Warning message prints only once |

### ✅ Completed: Phase 6A-6C (Interactive Shell)

| Component | Status | Notes |
|-----------|--------|-------|
| **Direct UEFI Boot** | ✅ Complete | No GRUB, standalone BOOTX64.EFI |
| **PS/2 Keyboard Driver** | ✅ Complete | IRQ1, scancode-to-ASCII, modifiers |
| **Framebuffer Console** | ✅ Complete | RGB565, PSF2 font (8x16), scrolling |
| **Process Management** | ✅ Complete | 256-slot table, round-robin scheduler |
| **Syscall Interface** | ✅ Complete | read, write, spawn, exit, getpid, yield |
| **VFS + Ramdisk** | ✅ Complete | Embedded ELF binaries (init, shell, hello) |
| **Interactive Shell** | ✅ Complete | C shell with Dracula theme, built-in commands |

---

## USB HID Keyboard Implementation

### Architecture

The USB HID keyboard driver uses a **polling-based architecture** for Phase 7A:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Keyboard Frontend                           │
│  ┌────────────┐   ┌────────────┐   ┌──────────────────────────┐   │
│  │    USB     │   │   PS/2      │   │     None (Halt)          │   │
│  │  (Tier-1)  │   │ (Fallback)  │   │  (No keyboard error)    │   │
│  └─────┬──────┘   └──────┬─────┘   └──────────────────────────┘   │
│        │                 │                                            │
└────────┼─────────────────┼────────────────────────────────────────┘
         │                 │
         ▼                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     USB HID Stack                                 │
│  ┌──────────┐  ┌──────────────────┐  ┌──────────┐  ┌───────────┐   │
│  │   pci    │→ │  xhci or ehci    │→ │  device  │→ │    hid    │   │
│  │ (scan)   │  │ (detect& init)   │  │ (enum)   │  │ (parse)   │   │
│  └──────────┘  └──────────────────┘  └──────────┘  └───────────┘   │
│       │              │                      │             │          │
│       ▼              ▼                      ▼             ▼          │
│   Scan PCI   xHCI/EHCI 2.0/3.0   Enumerate     Parse 8-byte       │
│   for USB    controller init    HID device   keyboard report      │
└─────────────────────────────────────────────────────────────────────┘
```

### File Structure

```
src/usb/
├── mod.rs          # USB module declarations, error types
├── pci.rs          # PCI configuration space scanning (xHCI, EHCI, UHCI, OHCI)
├── xhci.rs         # xHCI controller (USB 3.0), transfer rings, event rings
├── ehci.rs         # EHCI controller (USB 2.0), operational registers
├── trb.rs          # TRB structures (Normal, Setup, Status, Link, Event)
├── device.rs       # USB device enumeration, HID keyboard detection
└── hid.rs          # HID Boot Protocol keyboard parsing

src/keyboard/
├── mod.rs          # Backend selection (USB → PS/2 → None), warning flag
├── ps2.rs          # Legacy PS/2 driver (IRQ1)
└── usb.rs          # USB keyboard adapter layer
```

### xHCI Interrupt Transfer Flow

```rust
// 1. Check if controller and keyboard are available
let controller = xhci::controller()?;
let keyboard = device::get_hid_keyboard()?;

// 2. Create Normal TRB for interrupt IN transfer
let trb = NormalTrb {
    data_ptr: &HID_REPORT_BUFFER as *const u8 as u64,
    status: 8,  // TRB length (8 bytes)
    control: (1 << 10) | (1 << 16) | (1 << 5),  // Type=Normal, IOC, ISP
};

// 3. Enqueue TRB to transfer ring
controller.transfer_ring.enqueue(&trb)?;

// 4. Ring doorbell for endpoint
controller.ring_doorbell(slot_id, dci, 0);

// 5. Poll for event completion
while timeout > 0 {
    if let Some(event) = controller.poll_events() {
        return event.status & 0x00FFFFFF;  // Transfer length
    }
}
```

### HID Keycode Translation

256-entry translation table maps USB HID keycodes to ASCII:
- 0x04-0x1D: a-z (lowercase)
- 0x1E-0x27: 1-0, Enter, Esc, Backspace, Tab
- 0x2D-0x38: Symbols (-, =, [, ], \, ;, ', `, ,, ., /)
- Shift modifier handling for uppercase and shifted symbols

### Keyboard Backend Selection

```rust
pub fn init() {
    unsafe {
        // Reset warning flag
        NO_KEYBOARD_WARNING_SHOWN = false;

        // First try USB (xHCI > EHCI priority)
        if let Ok(()) = usb::init() {
            KEYBOARD_BACKEND = KeyboardBackend::Usb;
            crate::framebuffer::write_str("[USB KBD] ");
            return;
        }

        // Fall back to PS/2 (legacy hardware)
        if ps2::controller_present() {
            KEYBOARD_BACKEND = KeyboardBackend::Ps2;
            crate::framebuffer::write_str("[PS/2] ");
        } else {
            KEYBOARD_BACKEND = KeyboardBackend::None;
            crate::framebuffer::write_str("[NO KEYBOARD] ");
        }
    }
}
```

---

## Testing in QEMU

### USB Keyboard Test (xHCI - USB 3.0)

```bash
qemu-system-x86_64 \
  -bios /usr/share/OVMF/OVMF.fd \
  -drive file=rustica-live-amd64-0.1.0.img,format=raw \
  -m 512M \
  -machine q35 \
  -device qemu-xhci,id=xhci \
  -device usb-kbd,bus=xhci.0 \
  -serial stdio \
  -display gtk
```

**Important:** The `-device qemu-xhci` explicitly adds an xHCI controller, which is required for USB keyboard support in QEMU.

### USB Keyboard Test (EHCI - USB 2.0)

```bash
qemu-system-x86_64 \
  -bios /usr/share/OVMF/OVMF.fd \
  -drive file=rustica-live-amd64-0.1.0.img,format=raw \
  -m 512M \
  -machine q35 \
  -device usb-ehci,id=ehci \
  -device usb-kbd,bus=ehci.0 \
  -serial stdio \
  -display gtk
```

### PS/2 Keyboard Test (Legacy)

```bash
qemu-system-x86_64 \
  -drive format=raw,file=fat:rw:esp,if=ide \
  -bios /usr/share/OVMF/OVMF.fd \
  -machine type=pc \
  -device i8042
```

---

## Phase 7B TODO (Future Work)

The following features are stubs or simplified implementations that will be completed in Phase 7B:

1. **Full USB Enumeration** - Currently assumes keyboard is on slot 1, endpoint 1
   - Enable Slot command TRB
   - GET_DESCRIPTOR control transfers
   - SET_ADDRESS control transfers
   - Configuration descriptor parsing

2. **Endpoint Configuration** - Uses hardcoded endpoint address 0x81
   - Parse interface descriptor
   - Parse endpoint descriptor
   - Configure transfer rings per endpoint

3. **MSI/MSI-X Interrupts** - Currently polling only
   - Configure MSI for xHCI
   - Event-driven transfers instead of polling

4. **HID Report Descriptor Parsing** - Uses Boot Protocol
   - Full HID report descriptor parsing
   - Support for non-standard keyboards

---

## Directory Structure

```
/var/www/rustux.com/prod/loader/kernel-efi/
├── src/
│   ├── main.rs           # Kernel entry point, boot phases
│   ├── console.rs        # UEFI/framebuffer console abstraction
│   ├── framebuffer.rs    # RGB565 framebuffer with PSF2 font
│   ├── runtime.rs        # Interrupt handling, IDT, LAPIC, IOAPIC
│   ├── keyboard/         # Keyboard drivers
│   │   ├── mod.rs        # Backend selection, circular buffer
│   │   ├── ps2.rs        # PS/2 driver (IRQ1, legacy)
│   │   └── usb.rs        # USB keyboard adapter
│   ├── usb/              # USB stack (NEW)
│   │   ├── mod.rs        # Module declarations, error types
│   │   ├── pci.rs        # PCI scanning (xHCI, EHCI)
│   │   ├── xhci.rs       # xHCI controller (USB 3.0)
│   │   ├── ehci.rs       # EHCI controller (USB 2.0)
│   │   ├── trb.rs        # TRB structures
│   │   ├── device.rs     # Device enumeration, HID detection
│   │   └── hid.rs        # HID report parsing
│   ├── acpi.rs           # ACPI table parsing (MADT for IRQ routing)
│   ├── shell.rs          # Interactive shell
│   └── syscall.rs         # System call handlers
└── target/x86_64-unknown-uefi/release/
    └── rustux-kernel-efi.efi   # UEFI application
```

---

## Keyboard Support Matrix

| Keyboard Type | Connection | Method | Status |
|---------------|------------|--------|--------|
| **PS/2** | Physical port 0x60/0x64 | IRQ1 (interrupt) | ✅ Works (QEMU + real HW) |
| **USB HID** | xHCI controller (USB 3.0) | Polling | ✅ Works (QEMU) |
| **USB HID** | EHCI controller (USB 2.0) | Polling | ✅ Basic support added |
| **USB HID** | xHCI/EHCI | MSI/MSI-X | ⏳ Phase 7B |

**Honest Detection:**
- `[USB KBD]` - xHCI or EHCI controller found, USB keyboard detected
- `[PS/2]` - PS/2 controller present, USB not found
- `[NO KEYBOARD]` - Neither USB nor PS/2 detected (system continues in display-only mode)

**Known Issues (January 26, 2026):**
1. **No Repeated Warnings** - Fixed! The "No keyboard attached" message now prints only once
2. **EHCI Support Added** - Basic EHCI controller detection and initialization for USB 2.0
3. **QEMU Testing** - Use `-device qemu-xhci` for testing USB keyboard in QEMU
4. **Display-Only CLI Mode** - When `[NO KEYBOARD]` is detected, the shell runs in display-only mode
   - Commands: `help`, `clear`, `mem`, `kbd`, `ps`, `exit` work for viewing system state
   - No interactive input possible without keyboard hardware

---

## Technical Details

### xHCI Controller Registers

| Register Offset | Name | Purpose |
|-----------------|------|---------|
| 0x00 | CAPLENGTH | Capability register length |
| 0x18 | RTSOFF | Runtime register offset |
| 0x1C | DBOFF | Doorbell offset |
| 0x00 | USBCMD | USB command (RUN, HCRST) |
| 0x04 | USBSTS | USB status (HCH, CNR) |
| 0x30 | DCBAAP | Device context base array |
| 0x38 | CONFIG | Max device slots |

### TRB Types

| Type | Value | Purpose |
|------|-------|---------|
| Normal | 1 | Bulk/interrupt transfers |
| Setup Stage | 2 | Control setup |
| Data Stage | 3 | Control data |
| Status Stage | 4 | Control status |
| Link | 6 | Ring segmentation |
| Transfer Event | 32 | Transfer completion |
| Command Completion | 33 | Command completion |

### USB HID Boot Protocol Report

```rust
#[repr(C, packed)]
pub struct KeyboardReport {
    pub modifier: u8,      // Ctrl, Shift, Alt, Gui (L+R)
    pub reserved: u8,
    pub keycodes: [u8; 6], // Up to 6 simultaneous keys
}
```

---

## Build Instructions

```bash
cd /var/www/rustux.com/prod/loader/kernel-efi

# Build UEFI kernel
cargo build --release --target x86_64-unknown-uefi

# Output: target/x86_64-unknown-uefi/release/rustux-kernel-efi.efi
```

**Requirements:**
- Rust nightly
- `x86_64-unknown-uefi` target
- UEFI development environment

---

## System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| **Architecture** | x86_64 (AMD64) | x86_64 (AMD64) |
| **Boot** | UEFI 2.0 | UEFI 2.3+ |
| **RAM** | 512 MB | 1 GB |
| **Storage** | 128 MB | 4 GB |
| **Input** | USB HID (xHCI) or PS/2 | Both supported |

---

## Documentation

- **BUILD.md** - Build instructions
- **IMAGE.md** - System architecture
- **PLAN.md** - Development roadmap

---

## Git Repository

- **https://github.com/gitrustux/rustux**

---

## License

MIT License - See LICENSE file for details.

---

*Last Updated: January 26, 2026*
**Status:** Phase 7A IN PROGRESS - USB HID Keyboard with xHCI/EHCI Support (Polling)
**Added:** EHCI controller support, no repeated keyboard warnings, QEMU testing instructions
