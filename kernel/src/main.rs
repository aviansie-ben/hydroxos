#![no_main]
#![no_std]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_harness_main"]
#![test_runner(hydroxos_kernel::test_util::run_tests)]

extern crate alloc;

use core::panic::PanicInfo;

use bootloader::{entry_point, BootInfo};

#[cfg(not(test))]
entry_point!(kernel_main);

#[cfg(test)]
entry_point!(test_main);

#[allow(dead_code)]
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use hydroxos_kernel::log;

    unsafe {
        hydroxos_kernel::init_phase_1(boot_info);
        hydroxos_kernel::init_phase_2();
    };

    log!(Info, "kernel", "Done booting");
    show_command_prompt();
    hydroxos_kernel::arch::halt();
}

fn show_command_prompt() {
    use dyn_dyn::dyn_dyn_cast;
    use hydroxos_kernel::cmd::show_debug_console;
    use hydroxos_kernel::io::dev::{self, Device};
    use hydroxos_kernel::io::tty::Tty;

    let vt = dyn_dyn_cast!(move Device => Tty, dev::get_device_by_name("::serial0").ok().unwrap()).unwrap();
    show_debug_console(vt.dev());
}

#[cfg(test)]
fn test_main(_: &'static BootInfo) -> ! {
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
