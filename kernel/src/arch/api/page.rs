use super::PhysAddr;
use crate::virtual_alloc::VirtualAllocator;

pub const PAGE_SIZE: usize = 4096;

pub fn get_phys_mem_ptr<T>(phys_addr: PhysAddr) -> *const T {
    unimplemented!()
}

pub fn get_phys_mem_ptr_mut<T>(phys_addr: PhysAddr) -> *mut T {
    unimplemented!()
}

pub unsafe fn get_phys_mem_addr<T>(ptr: *const T) -> PhysAddr {
    unimplemented!()
}

pub struct AddressSpace;

impl AddressSpace {
    pub unsafe fn new_kernel() -> AddressSpace {
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
}
