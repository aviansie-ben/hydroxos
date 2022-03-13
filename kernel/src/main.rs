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
    use hydroxos_kernel::arch::page::AddressSpace;
    use hydroxos_kernel::log;
    use hydroxos_kernel::virtual_alloc::VirtualAllocRegion;

    unsafe {
        hydroxos_kernel::init_phase_1(boot_info);
        hydroxos_kernel::init_phase_2();
    };

    let addr = AddressSpace::kernel().virtual_alloc().alloc(8192).unwrap().start();
    unsafe {
        AddressSpace::kernel()
            .virtual_alloc()
            .free(VirtualAllocRegion::new(addr, addr + 4096u64));
        AddressSpace::kernel()
            .virtual_alloc()
            .free(VirtualAllocRegion::new(addr + 4096u64, addr + 8192u64));
    }

    for region in AddressSpace::kernel().virtual_alloc().free_regions() {
        log!(
            Debug,
            "kernel",
            "Free region: (0x{:016x}, 0x{:016x})",
            region.start().as_u64(),
            region.end().as_u64()
        );
    }

    log!(Info, "kernel", "Done booting");
    hydroxos_kernel::arch::halt();
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
