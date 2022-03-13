#![allow(dead_code)]
#![allow(unused_variables)]

use core::ops::{Add, AddAssign, Sub};

use bootloader::BootInfo;

pub mod interrupt;
pub mod page;
pub mod regs;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub const fn new(val: u64) -> PhysAddr {
        unimplemented!()
    }

    pub const fn zero() -> PhysAddr {
        unimplemented!()
    }

    pub const fn as_u64(self) -> u64 {
        unimplemented!()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(u64);

impl VirtAddr {
    pub fn new(val: u64) -> VirtAddr {
        unimplemented!()
    }

    pub const fn new_truncate(val: u64) -> VirtAddr {
        unimplemented!()
    }

    pub const fn zero() -> VirtAddr {
        unimplemented!()
    }

    pub const fn as_u64(self) -> u64 {
        unimplemented!()
    }

    pub fn is_aligned(self, align: u64) -> bool {
        unimplemented!()
    }
}

impl Sub for VirtAddr {
    type Output = u64;

    fn sub(self, rhs: Self) -> Self::Output {
        unimplemented!()
    }
}

impl Add<usize> for VirtAddr {
    type Output = VirtAddr;

    fn add(self, rhs: usize) -> Self::Output {
        unimplemented!()
    }
}

impl AddAssign<usize> for VirtAddr {
    fn add_assign(&mut self, rhs: usize) {
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
