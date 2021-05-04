use bootloader::BootInfo;

use crate::io::vt::{VirtualTerminalDisplay};

pub mod cpuid;
pub mod dev;
pub mod gdt;
pub mod idt;
pub mod page;
pub mod pic;

pub unsafe fn create_primary_display(_: &'static BootInfo) -> VirtualTerminalDisplay {
    use self::dev::vgabuf::TextBuffer;

    VirtualTerminalDisplay::VgaText(TextBuffer::new(0xb8000 as *mut u8, 80, 25))
}

unsafe fn init_sse() {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

    Cr0::write(Cr0::read() & !Cr0Flags::EMULATE_COPROCESSOR | Cr0Flags::MONITOR_COPROCESSOR);
    Cr4::write(Cr4::read() | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
    asm!("fninit");
}

pub unsafe fn init_phase_1(boot_info: &'static BootInfo) {
    use x86_64::instructions::interrupts;

    cpuid::init_bsp();

    page::init_phys_mem_base(boot_info.physical_memory_offset as *mut u8);
    idt::init_bsp();
    pic::remap_pic(idt::IRQS_START, idt::IRQS_START + 0x8);
    pic::mask_all_irqs();
    interrupts::enable();

    init_sse();
    crate::sched::task::x86_64::init_xsave();
}
