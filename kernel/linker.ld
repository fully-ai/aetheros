/* AetherOS x86_64 Higher-Half Kernel Linker Script (-2 GiB virtual base) */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS
{
    limine_requests PT_LOAD FLAGS(6); /* RW */
    text            PT_LOAD FLAGS(5); /* RX */
    rodata          PT_LOAD FLAGS(4); /* R  */
    data            PT_LOAD FLAGS(6); /* RW */
}

SECTIONS
{
    . = 0xffffffff80000000;

    __kernel_start = .;

    .limine_requests : ALIGN(4096) {
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :limine_requests

    . = ALIGN(4096);
    __text_start = .;
    .text : {
        *(.text .text.*)
    } :text
    . = ALIGN(4096);
    __text_end = .;

    __rodata_start = .;
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata
    . = ALIGN(4096);
    __rodata_end = .;

    __data_start = .;
    .data : {
        *(.data .data.*)
    } :data

    __bss_start = .;
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data
    . = ALIGN(4096);
    __bss_end = .;
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
