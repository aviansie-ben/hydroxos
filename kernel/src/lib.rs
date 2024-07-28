#![no_std]
#![cfg_attr(test, no_main)]
#![feature(asm_const)]
#![feature(alloc_error_handler)]
#![feature(coerce_unsized)]
#![feature(const_mut_refs)]
#![feature(const_replace)]
#![feature(custom_test_frameworks)]
#![feature(maybe_uninit_uninit_array)]
#![feature(naked_functions)]
#![feature(negative_impls)]
#![feature(ptr_metadata)]
#![feature(slice_ptr_get)]
#![feature(thread_local)]
#![feature(try_blocks)]
#![feature(unsize)]
#![allow(incomplete_features)]
#![feature(specialization)]
#![reexport_test_harness_main = "test_harness_main"]
#![test_runner(crate::test_util::run_tests)]
#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::new_without_default)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::unnecessary_cast)] // Incorrectly catches cases where pointee type is unknown

extern crate alloc;

use bootloader::BootInfo;

// Declared first so we can use the log! macro in all other modules
pub mod log;

pub mod arch;
pub mod cmd;
pub mod early_alloc;
pub mod frame_alloc;
pub mod io;
pub mod panic;
pub mod sched;
pub mod sync;
pub mod test_util;
pub mod util;
pub mod virtual_alloc;

pub unsafe fn init_phase_1(boot_info: &'static BootInfo) {
    early_alloc::init();
    arch::init_phase_1(boot_info);

    frame_alloc::init(boot_info);
    log::init(io::vt::get_global_manager().dev().get_terminal(0).unwrap());
}

pub unsafe fn init_phase_2() {
    use crate::arch::page::PAGE_SIZE;
    use crate::frame_alloc::FrameAllocator;
    use crate::io::dev::log_device_tree;

    log!(Info, "kernel", "Booting HydroxOS v{}", env!("CARGO_PKG_VERSION"));
    log!(
        Debug,
        "kernel",
        "Detected {} MiB memory, {} MiB free",
        frame_alloc::num_total_frames() * PAGE_SIZE / (1024 * 1024),
        frame_alloc::get_allocator().num_frames_available() * PAGE_SIZE / (1024 * 1024)
    );

    arch::init_phase_2();
    sched::init();

    log_device_tree();
}

#[cfg(test)]
mod test {
    use core::panic::PanicInfo;

    use bootloader::{entry_point, BootInfo};

    entry_point!(test_main);

    pub fn test_main(boot_info: &'static BootInfo) -> ! {
        unsafe {
            crate::init_phase_1(boot_info);
            crate::test_util::init_test_log();
            crate::init_phase_2();
        };
        crate::test_harness_main();
        crate::arch::halt();
    }

    #[panic_handler]
    fn panic(info: &PanicInfo) -> ! {
        crate::test_util::handle_test_panic(info);
    }
}
