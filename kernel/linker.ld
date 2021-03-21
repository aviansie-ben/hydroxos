OUTPUT_FORMAT("elf64-x86-64")
ENTRY(_start)
SECTIONS
{
  . = 0xFFFFFF0000000000;
  .text : { *(.text*) }

  . = ALIGN(0x1000);
  .rodata : { *(.rodata*) }
  .eh_frame_hdr : { *(.eh_frame_hdr) }
  .eh_frame : { *(.eh_frame*) }

  . = ALIGN(0x1000);
  .tdata : { *(.tdata*) }
  .tbss : { *(.tbss*) }
  .got : { *(.got) }

  . = ALIGN(0x1000);
  .data : { *(.data*) }
  .bss : { *(.bss*) *(COMMON) }
}
