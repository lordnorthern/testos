ENTRY(_start)

SECTIONS
{
    /* RPi2 firmware loads kernel7.img at 0x8000 */
    . = 0x8000;

    .text :
    {
        KEEP(*(.text._start))
        *(.text .text.*)
    }

    .rodata ALIGN(4) :
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4) :
    {
        *(.data .data.*)
    }

    . = ALIGN(4);
    __bss_start = .;

    .bss (NOLOAD) :
    {
        *(.bss .bss.*)
        *(COMMON)
    }

    . = ALIGN(4);
    __bss_end = .;

    /DISCARD/ : { *(.comment) *(.ARM.exidx*) *(.ARM.extab*) *(.note*) }
}
