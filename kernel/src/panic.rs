use core::panic::PanicInfo;

#[cfg(not(feature = "check_arch_api"))]
pub fn show_panic_crash_screen(info: &PanicInfo) -> ! {
    use core::fmt::Write;

    use crate::arch::x86_64::dev::vgabuf::{Color, VgaTextBuffer, Writer};

    crate::mem::set_use_early_alloc(true);

    let mut vga_buf = unsafe { VgaTextBuffer::for_primary_display() };
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
