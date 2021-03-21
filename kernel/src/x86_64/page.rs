use crate::util::SharedUnsafeCell;

pub const PAGE_SIZE: usize = 4096;

static PHYS_MEM_BASE: SharedUnsafeCell<*mut u8> = SharedUnsafeCell::new(core::ptr::null_mut());

pub fn init_phys_mem_base(phys_mem_base: *mut u8) {
    unsafe {
        assert_eq!(core::ptr::null_mut(), *PHYS_MEM_BASE.get());
        *PHYS_MEM_BASE.get() = phys_mem_base;
    };
}

pub fn get_phys_mem_base() -> *mut u8 {
    unsafe {
        let phys_mem_base = *PHYS_MEM_BASE.get();

        assert_ne!(core::ptr::null_mut(), phys_mem_base);
        phys_mem_base
    }
}

pub fn get_phys_mem_ptr<T>(phys_addr: x86_64::PhysAddr) -> *const T {
    get_phys_mem_base().wrapping_offset(phys_addr.as_u64() as isize) as *const T
}

pub fn get_phys_mem_ptr_mut<T>(phys_addr: x86_64::PhysAddr) -> *mut T {
    get_phys_mem_ptr::<T>(phys_addr) as *mut T
}
