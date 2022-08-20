use core::panic::PanicInfo;

#[cfg(not(feature = "check_arch_api"))]
pub fn show_panic_crash_screen(info: &PanicInfo) -> ! {
    use core::fmt::Write;

    use x86_64::PhysAddr;

    use crate::arch::page::get_phys_mem_ptr_mut;
    use crate::arch::x86_64::dev::vgabuf::{Color, VgaTextBuffer, Writer};

    let mut vga_buf = unsafe { VgaTextBuffer::new(get_phys_mem_ptr_mut(PhysAddr::new(0xb8000)), 80, 25) };
    let mut w = Writer::new(&mut vga_buf);

    w.set_color(Color::White, Color::Red);
    w.clear();

    let _ = write!(w, "{}", info);

    loop {
        x86_64::instructions::hlt();
    }
}

#[cfg(feature = "check_arch_api")]
pub fn show_panic_crash_screen(_info: &PanicInfo) -> ! {
    crate::arch::halt()
}

#[alloc_error_handler]
fn alloc_error_handler(layout: alloc::alloc::Layout) -> ! {
    panic!("Failed to allocate {:?}", layout);
}
