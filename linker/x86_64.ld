OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

KERNEL_PHYS_BASE = 0x00200000;
KERNEL_VIRT_BASE = 0xffffffff80000000;
KERNEL_STACK_SIZE = 0x00010000;

PHDRS
{
    text     PT_LOAD FLAGS(5); /* RX */
    rodata   PT_LOAD FLAGS(4); /* R  */
    requests PT_LOAD FLAGS(6); /* RW, patched by Limine */
    data     PT_LOAD FLAGS(6); /* RW */
}

SECTIONS
{
    . = KERNEL_VIRT_BASE + KERNEL_PHYS_BASE;
    __kernel_start = .;

    .text : AT(ADDR(.text) - KERNEL_VIRT_BASE)
    {
        __text_start = .;
        KEEP(*(.text._start))
        *(.text .text.*)
        __text_end = .;
    } :text

    . = ALIGN(4K);
    .rodata : AT(ADDR(.rodata) - KERNEL_VIRT_BASE)
    {
        __rodata_start = .;
        *(.rodata .rodata.*)
        __rodata_end = .;
    } :rodata

    . = ALIGN(4K);
    .requests_start_marker : AT(ADDR(.requests_start_marker) - KERNEL_VIRT_BASE)
    {
        __limine_requests_start = .;
        KEEP(*(.requests_start_marker))
    } :requests

    .requests : AT(ADDR(.requests) - KERNEL_VIRT_BASE)
    {
        KEEP(*(.requests))
    } :requests

    .requests_end_marker : AT(ADDR(.requests_end_marker) - KERNEL_VIRT_BASE)
    {
        KEEP(*(.requests_end_marker))
        __limine_requests_end = .;
    } :requests

    . = ALIGN(4K);
    .data : AT(ADDR(.data) - KERNEL_VIRT_BASE)
    {
        __data_start = .;
        *(.data .data.*)
        __data_end = .;
    } :data

    . = ALIGN(4K);
    .bss (NOLOAD) : AT(ADDR(.bss) - KERNEL_VIRT_BASE)
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    } :data

    . = ALIGN(4K);
    .bootstrap_stack (NOLOAD) : AT(ADDR(.bootstrap_stack) - KERNEL_VIRT_BASE)
    {
        . = ALIGN(16);
        __stack_bottom = .;
        . += KERNEL_STACK_SIZE;
        . = ALIGN(16);
        __stack_top = .;
    } :data

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.eh_frame_hdr .eh_frame_hdr.*)
        *(.note .note.*)
        *(.comment)
    }
}
