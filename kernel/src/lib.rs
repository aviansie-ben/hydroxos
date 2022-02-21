#![no_std]
#![cfg_attr(test, no_main)]
#![feature(asm_const)]
#![feature(asm_sym)]
#![feature(alloc_error_handler)]
#![feature(const_fn_trait_bound)]
#![feature(const_mut_refs)]
#![feature(custom_test_frameworks)]
#![feature(exclusive_range_pattern)]
#![feature(naked_functions)]
#![feature(negative_impls)]
#![feature(slice_ptr_len)]
#![feature(thread_local)]
#![allow(incomplete_features)]
#![feature(specialization)]
#![reexport_test_harness_main = "test_harness_main"]
#![test_runner(crate::test_util::run_tests)]

#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::new_without_default)]
#![allow(clippy::result_unit_err)]

extern crate alloc;

use bootloader::BootInfo;

// Declared first so we can use the log! macro in all other modules
pub mod log;

pub mod arch;
pub mod early_alloc;
pub mod frame_alloc;
pub mod io;
pub mod panic;
pub mod sched;
pub mod sync;
pub mod test_util;
pub mod util;

pub unsafe fn init_phase_1(boot_info: &'static BootInfo) {
    use crate::arch::page::PAGE_SIZE;
    use crate::frame_alloc::FrameAllocator;

    early_alloc::init();
    arch::init_phase_1(boot_info);

    let num_frames = frame_alloc::init(boot_info);

    log::init(io::vt::get_terminal(0).unwrap());

    log!(Info, "kernel", "Booting HydroxOS v{}", env!("CARGO_PKG_VERSION"));
    log!(
        Debug,
        "kernel",
        "Detected {} MiB memory, {} MiB free",
        num_frames * PAGE_SIZE / (1024 * 1024),
        frame_alloc::get_allocator().num_frames_available() * PAGE_SIZE / (1024 * 1024)
    );
}

pub unsafe fn init_phase_2() {
    arch::init_phase_2();
    sched::init();
}

#[cfg(test)]
mod test {
    use core::panic::PanicInfo;

    use bootloader::{entry_point, BootInfo};

    entry_point!(test_main);

    pub fn test_main(boot_info: &'static BootInfo) -> ! {
        unsafe {
            crate::init_phase_1(boot_info);
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
