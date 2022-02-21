use super::PhysAddr;

pub const PAGE_SIZE: usize = 8;

pub fn get_phys_mem_ptr<T>(phys_addr: PhysAddr) -> *const T {
    unimplemented!()
}

pub fn get_phys_mem_ptr_mut<T>(phys_addr: PhysAddr) -> *mut T {
    unimplemented!()
}

pub unsafe fn get_phys_mem_addr<T>(ptr: *const T) -> PhysAddr {
    unimplemented!()
}
