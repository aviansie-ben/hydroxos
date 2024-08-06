use core::marker::PhantomData;

use bitflags::bitflags;

use super::{PhysAddr, VirtAddr};
use crate::mem::virt::VirtualAllocator;
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;

pub const PAGE_SIZE: usize = 4096;
pub const IS_PHYS_MEM_ALWAYS_MAPPED: bool = true;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PageFlags: u16 {
        const USER = 0x1;
        const WRITEABLE = 0x2;
        const EXECUTABLE = 0x4;
    }
}

#[derive(Debug)]
pub struct PhysMemPtr<T: ?Sized> {
    _data: PhantomData<*mut T>,
}

impl<T: ?Sized> PhysMemPtr<T> {
    pub fn ptr(&self) -> *mut T {
        unimplemented!()
    }

    pub fn phys_addr(&self) -> PhysAddr {
        unimplemented!()
    }

    pub fn into_raw(self) -> *mut T {
        unimplemented!()
    }

    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        unimplemented!()
    }
}

impl<T: ?Sized> Drop for PhysMemPtr<T> {
    fn drop(&mut self) {
        unimplemented!()
    }
}

pub fn get_phys_mem_ptr<T>(phys_addr: PhysAddr) -> PhysMemPtr<T> {
    unimplemented!()
}

pub fn get_phys_mem_ptr_slice<T>(phys_addr: PhysAddr, len: usize) -> PhysMemPtr<[T]> {
    unimplemented!()
}

pub struct AddressSpace;

impl AddressSpace {
    pub unsafe fn new_kernel() -> AddressSpace {
        unimplemented!()
    }

    pub fn kernel() -> UninterruptibleSpinlockGuard<'static, AddressSpace> {
        unimplemented!()
    }

    pub fn new() -> AddressSpace {
        unimplemented!()
    }

    pub(crate) unsafe fn init_kernel_virtual_alloc(&mut self) {
        unimplemented!()
    }

    pub fn virtual_alloc(&mut self) -> &mut VirtualAllocator {
        unimplemented!()
    }

    pub fn get_page(&self, addr: VirtAddr) -> Option<(PhysAddr, PageFlags)> {
        unimplemented!()
    }

    pub fn set_page_user(&mut self, addr: VirtAddr, mapping: Option<(PhysAddr, PageFlags)>) {
        unimplemented!()
    }

    pub unsafe fn set_page_kernel(&mut self, addr: VirtAddr, mapping: Option<(PhysAddr, PageFlags)>) {
        unimplemented!()
    }
}
