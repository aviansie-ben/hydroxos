use alloc::boxed::Box;
use core::arch::asm;
use core::ptr;

use bootloader::BootInfo;
pub use x86_64::{PhysAddr, VirtAddr};

use crate::arch::dev::vgabuf::VgaTextBufferDevice;
use crate::io::dev::DeviceNode;
use crate::options;
use crate::util::OneShotManualInit;

pub mod cpuid;
pub mod dev;
pub mod gdt;
pub mod interrupt;
pub mod page;
pub mod pic;
pub mod regs;

static KERNEL_FS_BASE: OneShotManualInit<u64> = OneShotManualInit::uninit();

unsafe fn init_sse() {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

    Cr0::write(Cr0::read() & !Cr0Flags::EMULATE_COPROCESSOR | Cr0Flags::MONITOR_COPROCESSOR);
    Cr4::write(Cr4::read() | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
    asm!("fninit");
}

unsafe fn init_bootstrap_tls(boot_info: &BootInfo) {
    if let Some(tls_template) = boot_info.tls_template() {
        assert!(tls_template.file_size <= tls_template.mem_size);
        assert_eq!(0, tls_template.mem_size & 0xf);

        let tls = crate::mem::early::alloc(tls_template.mem_size as usize + 8, 16);
        let tib = tls.add(tls_template.mem_size as usize);

        ptr::write_bytes(tls, 0, tls_template.mem_size as usize);
        ptr::copy_nonoverlapping(tls_template.start_addr as *mut u8, tls, tls_template.file_size as usize);
        ptr::write::<*mut u8>(tib as *mut *mut u8, tib as *mut u8);

        x86_64::registers::model_specific::Msr::new(0xc0000100).write(tib as u64);
        KERNEL_FS_BASE.set(tib as u64);
    };
}

pub(crate) unsafe fn init_phase_1(boot_info: &BootInfo) {
    page::init_phys_mem_base(boot_info.physical_memory_offset as *mut u8);
    init_bootstrap_tls(boot_info);
    cpuid::init_bsp();

    crate::io::dev::init_device_root();

    let serial = dev::serial::init();

    if options::get().get_flag("serial_log").unwrap_or(false) {
        crate::log::add_tty(serial);
    }

    let vga_text = crate::io::dev::device_root()
        .dev()
        .add_device(DeviceNode::new(Box::from("vgatext"), VgaTextBufferDevice::for_primary_display()));
    crate::io::vt::init(vga_text);

    gdt::init();
    interrupt::init_bsp();
    pic::remap_pic(interrupt::IRQS_START, interrupt::IRQS_START + 0x8);
    pic::mask_all_irqs();

    init_sse();
    regs::init_xsave();
}

pub(crate) unsafe fn init_phase_2() {
    page::init_kernel_addrspace();
    crate::mem::set_use_early_alloc(false);
    dev::ps2::init();
}

#[naked]
unsafe extern "C" fn idle() {
    asm!(
        "sti",
        "hlt",
        "jmp {}",
        sym idle,
        options(noreturn)
    );
}

pub fn halt() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
