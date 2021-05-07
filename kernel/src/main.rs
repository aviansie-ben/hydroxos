#![no_main]
#![no_std]

#![feature(custom_test_frameworks)]

#![reexport_test_harness_main = "test_harness_main"]
#![test_runner(test_os::test_util::run_tests)]

use core::panic::PanicInfo;
use bootloader::{BootInfo, entry_point};

entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use core::fmt::Write;
    use test_os::{early_alloc, io, sched, x86_64};

    unsafe {
        early_alloc::init();
        x86_64::init_phase_1(boot_info);
        sched::init();
    };

    writeln!(io::tty::TtyWriter::new(io::vt::get_terminal(0).unwrap().as_ref()), "{:#?} {:?}", boot_info, boot_info as *const BootInfo).unwrap();

    loop {
        ::x86_64::instructions::hlt();
    };
}

#[cfg(test)]
fn kernel_main(_: &'static BootInfo) -> ! {
    // We don't have any tests on the binary right now
    test_os::test_util::exit(0);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use x86_64::instructions::interrupts;
    use test_os::panic;

    interrupts::disable();
    panic::show_panic_crash_screen(info);
}
