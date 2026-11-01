// Copyright 2026 The Rustux Authors
//
// Time source: the CPU timestamp counter (TSC), calibrated once against
// the PIT (8254 channel 2, the PC speaker timer, which is not used for
// anything else). Gives microsecond delays and timeouts that mean the same
// on QEMU and on real hardware, unlike counted busy loops.

static mut TSC_PER_US: u64 = 0;

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack, preserves_flags));
    v
}

unsafe fn outb(port: u16, v: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags));
}

/// Read the timestamp counter
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe { core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    ((hi as u64) << 32) | lo as u64
}

/// Calibrate the TSC (about 10 ms). Safe to call more than once; only the
/// first call measures.
pub fn init() {
    unsafe {
        if TSC_PER_US != 0 {
            return;
        }
        // Channel 2 gate on, speaker output off
        let port61 = inb(0x61);
        outb(0x61, (port61 & !0x02) | 0x01);
        // Channel 2, lobyte/hibyte, mode 0 (interrupt on terminal count)
        outb(0x43, 0xB0);
        const PIT_HZ: u64 = 1_193_182;
        const MS: u64 = 10;
        let count = (PIT_HZ * MS / 1000) as u16;
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);

        let start = rdtsc();
        // OUT2 (port 0x61 bit 5) goes high when the count reaches 0
        let mut spins: u64 = 0;
        while inb(0x61) & 0x20 == 0 {
            spins += 1;
            if spins > 50_000_000 {
                break; // no working PIT: fall back below
            }
        }
        let end = rdtsc();
        outb(0x61, port61);

        let per_us = (end - start) / (MS * 1000);
        TSC_PER_US = if spins > 50_000_000 || per_us < 100 { 3000 } else { per_us };
    }
}

/// TSC ticks per microsecond (after [`init`])
pub fn tsc_per_us() -> u64 {
    unsafe {
        if TSC_PER_US == 0 {
            init();
        }
        TSC_PER_US
    }
}

/// Microseconds since an arbitrary point (boot)
pub fn now_us() -> u64 {
    rdtsc() / tsc_per_us()
}

/// Busy-wait for `us` microseconds
pub fn delay_us(us: u64) {
    let end = rdtsc() + us * tsc_per_us();
    while rdtsc() < end {
        core::hint::spin_loop();
    }
}

/// Busy-wait for `ms` milliseconds
pub fn delay_ms(ms: u64) {
    delay_us(ms * 1000);
}

/// A point in time `ms` milliseconds from now, for timeouts
#[derive(Clone, Copy)]
pub struct Deadline(u64);

impl Deadline {
    pub fn after_ms(ms: u64) -> Self {
        Deadline(rdtsc() + ms * 1000 * tsc_per_us())
    }

    pub fn expired(&self) -> bool {
        rdtsc() >= self.0
    }
}
