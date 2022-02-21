#![allow(unused_variables)]

use bootloader::BootInfo;

pub mod interrupt;
pub mod page;
pub mod regs;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PhysAddr(u64);

impl PhysAddr {
    // TODO: This should not be required!
    pub const fn new(val: u64) -> PhysAddr {
        unimplemented!()
    }

    pub const fn zero() -> PhysAddr {
        unimplemented!()
    }

    // TODO: This should not be required!
    pub const fn as_u64(self) -> u64 {
        unimplemented!()
    }
}

pub fn halt() -> ! {
    unimplemented!()
}

pub(crate) unsafe fn init_phase_1(boot_info: &BootInfo) {
    unimplemented!()
}

pub(crate) unsafe fn init_phase_2() {
    unimplemented!()
}
