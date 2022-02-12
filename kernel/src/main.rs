#![no_main]
#![no_std]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_harness_main"]
#![test_runner(hydroxos_kernel::test_util::run_tests)]

extern crate alloc;

use core::panic::PanicInfo;

use bootloader::{entry_point, BootInfo};

entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use core::fmt::Write;

    use hydroxos_kernel::frame_alloc::FrameAllocator;
    use hydroxos_kernel::{early_alloc, frame_alloc, io, log, sched, x86_64};

    unsafe {
        early_alloc::init();
        x86_64::init_phase_1(boot_info);

        let num_frames = frame_alloc::init(boot_info);

        log::init(io::vt::get_terminal(0).unwrap());

        log!(Info, "kernel", "Booting HydroxOS v{}", env!("CARGO_PKG_VERSION"));
        log!(
            Debug,
            "kernel",
            "Detected {} MiB memory, {} MiB free",
            num_frames * x86_64::page::PAGE_SIZE / (1024 * 1024),
            frame_alloc::get_allocator().num_frames_available() * x86_64::page::PAGE_SIZE / (1024 * 1024)
        );

        x86_64::init_phase_2(boot_info);

        sched::init();
    };

    log!(Info, "kernel", "Done booting");

    loop {
        ::x86_64::instructions::hlt();
    }
}

#[cfg(test)]
fn kernel_main(_: &'static BootInfo) -> ! {
    // We don't have any tests on the binary right now
    hydroxos_kernel::test_util::exit(0);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use hydroxos_kernel::panic;
    use x86_64::instructions::interrupts;

    interrupts::disable();
    panic::show_panic_crash_screen(info);
}
