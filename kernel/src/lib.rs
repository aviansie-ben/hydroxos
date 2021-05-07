#![no_std]
#![cfg_attr(test, no_main)]

#![feature(asm)]
#![feature(alloc_error_handler)]
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

extern crate alloc;

pub mod early_alloc;
pub mod future;
pub mod io;
pub mod panic;
pub mod frame_alloc;
pub mod sched;
pub mod util;
pub mod x86_64;
pub mod test_util;

#[cfg(test)]
mod test {
    use core::panic::PanicInfo;
    use bootloader::{BootInfo, entry_point};

    entry_point!(test_main);

    pub fn test_main(boot_info: &'static BootInfo) -> ! {
        unsafe {
            crate::early_alloc::init();
            crate::x86_64::init_phase_1(boot_info);
            crate::sched::init();
        };
        crate::test_harness_main();
        loop {};
    }

    #[panic_handler]
    fn panic(info: &PanicInfo) -> ! {
        crate::test_util::handle_test_panic(info);
    }
}
