use core::fmt::Write;
use core::panic::PanicInfo;

use crate::x86_64::dev::vgabuf::{Color, TextBuffer, Writer};

pub fn show_panic_crash_screen(info: &PanicInfo) -> ! {
    let mut vga_buf = unsafe { TextBuffer::new(0xb8000 as *mut u8, 80, 25) };
    let mut w = Writer::new(&mut vga_buf);

    w.set_color(Color::White, Color::Red);
    w.clear();

    let _ = write!(w, "{}", info);

    loop {}
}

#[alloc_error_handler]
fn alloc_error_handler(layout: alloc::alloc::Layout) -> ! {
    panic!("Failed to allocate {:?}", layout);
}
