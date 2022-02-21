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

// Declared first so we can use the log! macro in all other modules
pub mod log;

pub mod early_alloc;
pub mod frame_alloc;
pub mod io;
pub mod panic;
pub mod sched;
pub mod sync;
pub mod test_util;
pub mod util;
pub mod x86_64;

#[cfg(test)]
mod test {
    use core::panic::PanicInfo;

    use bootloader::{entry_point, BootInfo};

    entry_point!(test_main);

    pub fn test_main(boot_info: &'static BootInfo) -> ! {
        unsafe {
            crate::early_alloc::init();
            crate::x86_64::init_phase_1(boot_info);
            crate::frame_alloc::init(boot_info);
            crate::x86_64::init_phase_2(boot_info);
            crate::sched::init();
        };
        crate::test_harness_main();
        loop {
            ::x86_64::instructions::hlt();
        }
    }

    #[panic_handler]
    fn panic(info: &PanicInfo) -> ! {
        crate::test_util::handle_test_panic(info);
    }
}
