#![no_std]
#![no_main]

#![feature(asm)]
#![feature(alloc_error_handler)]
#![feature(custom_test_frameworks)]
#![feature(exclusive_range_pattern)]
#![feature(naked_functions)]
#![feature(slice_ptr_len)]

#![allow(incomplete_features)]
#![feature(specialization)]

#![reexport_test_harness_main = "test_harness_main"]
#![test_runner(crate::test_util::run_tests)]

extern crate alloc;

use core::panic::PanicInfo;
use bootloader::{BootInfo, entry_point};

pub mod early_alloc;
pub mod future;
pub mod io;
pub mod panic;
pub mod frame_alloc;
pub mod sched;
pub mod util;
pub mod x86_64;

#[cfg(test)]
pub mod test_util;

#[cfg(test)]
entry_point!(test_main);

#[cfg(test)]
pub fn test_main(boot_info: &'static BootInfo) -> ! {
    x86_64::page::init_phys_mem_base(boot_info.physical_memory_offset as *mut u8);
    test_harness_main();
    loop {};
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_util::handle_test_panic(info);
}

#[cfg(not(test))]
entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use core::fmt::Write;

    unsafe {
        early_alloc::init();
        io::vt::init(x86_64::create_primary_display(boot_info), 1);
        x86_64::init_phase_1(boot_info);
    };

    writeln!(io::tty::TtyWriter::new(io::vt::get_terminal(0).unwrap().as_ref()), "{:#?} {:?}", boot_info, boot_info as *const BootInfo).unwrap();

    loop {
        ::x86_64::instructions::hlt();
    };
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use ::x86_64::instructions::interrupts;

    interrupts::disable();
    panic::show_panic_crash_screen(info);
}
