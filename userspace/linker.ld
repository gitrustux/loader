/* Linker script for Rustux userspace programs */

ENTRY(_start)

SECTIONS {
    /* Program code and read-only data */
    . = 0x100000;  /* Start at 1MB */
    .text : {
        *(.text*)
        *(.rodata*)
    }

    /* Read-write data (initialized) */
    .data : {
        *(.data*)
    }

    /* Read-write data (uninitialized) */
    .bss : {
        *(.bss*)
        *(COMMON)
    }

    /* Discard unwind sections */
    /DISCARD/ : {
        *(.eh_frame*)
        *(.note.gnu.build-id)
    }
}
