use bootloader::BootInfo;

use crate::io::vt::{VirtualTerminalDisplay};

pub mod dev;
pub mod gdt;
pub mod idt;
pub mod page;
pub mod pic;

pub unsafe fn create_primary_display(_: &'static BootInfo) -> VirtualTerminalDisplay {
    use self::dev::vgabuf::TextBuffer;

    VirtualTerminalDisplay::VgaText(TextBuffer::new(0xb8000 as *mut u8, 80, 25))
}

pub unsafe fn init_phase_1(boot_info: &'static BootInfo) {
    use x86_64::instructions::interrupts;

    page::init_phys_mem_base(boot_info.physical_memory_offset as *mut u8);
    idt::init_bsp();
    pic::remap_pic(idt::IRQS_START, idt::IRQS_START + 0x8);
    pic::mask_all_irqs();
    interrupts::enable();
}
