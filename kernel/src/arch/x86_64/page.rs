use core::ptr;

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::PageTableFrameMapping;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{FrameDeallocator, MappedPageTable, PageTable, PageTableFlags, PageTableIndex, PhysFrame, Size4KiB};
use x86_64::PhysAddr;

use crate::frame_alloc::FrameAllocator;
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;
use crate::sync::UninterruptibleSpinlock;
use crate::util::SharedUnsafeCell;

pub const PAGE_SIZE: usize = 4096;

static PHYS_MEM_BASE: SharedUnsafeCell<*mut u8> = SharedUnsafeCell::new(core::ptr::null_mut());
static KERNEL_ADDRESS_SPACE: SharedUnsafeCell<UninterruptibleSpinlock<AddressSpace>> =
    SharedUnsafeCell::new(UninterruptibleSpinlock::new(unsafe { AddressSpace::from_ptr(PhysAddr::zero()) }));

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

pub fn get_phys_mem_ptr<T>(phys_addr: PhysAddr) -> *const T {
    get_phys_mem_base().wrapping_offset(phys_addr.as_u64() as isize) as *const T
}

pub fn get_phys_mem_ptr_mut<T>(phys_addr: PhysAddr) -> *mut T {
    get_phys_mem_ptr::<T>(phys_addr) as *mut T
}

pub unsafe fn get_phys_mem_addr<T>(ptr: *const T) -> PhysAddr {
    PhysAddr::new((ptr as *mut u8).offset_from(get_phys_mem_base()) as u64)
}

struct PhysPageTableFrameMapping;

unsafe impl PageTableFrameMapping for PhysPageTableFrameMapping {
    fn frame_to_pointer(&self, frame: PhysFrame) -> *mut PageTable {
        get_phys_mem_ptr_mut(frame.start_address()) as *mut PageTable
    }
}

struct PhysFrameDeallocator<'a, T: FrameAllocator>(&'a mut T);

impl<'a, T: FrameAllocator> FrameDeallocator<Size4KiB> for PhysFrameDeallocator<'a, T> {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        self.0.free_one(frame.start_address())
    }
}

pub struct AddressSpace(PhysAddr);

impl AddressSpace {
    pub fn kernel() -> UninterruptibleSpinlockGuard<'static, AddressSpace> {
        unsafe {
            let kernel_addrspace = (*KERNEL_ADDRESS_SPACE.get()).lock();

            assert_ne!(kernel_addrspace.0, PhysAddr::zero());
            kernel_addrspace
        }
    }

    pub const unsafe fn from_ptr(ptr: PhysAddr) -> AddressSpace {
        AddressSpace(ptr)
    }

    pub fn new() -> AddressSpace {
        unsafe {
            let mut addrspace = AddressSpace::from_ptr(crate::frame_alloc::get_allocator().alloc_one().unwrap());
            let mut l4_table = addrspace.as_page_table();
            let l4_table = l4_table.level_4_table();

            for i in 0..256 {
                let i = PageTableIndex::new(i);
                l4_table[i] = PageTableEntry::new();
            }

            {
                let mut kernel_addrspace = AddressSpace::kernel();
                let mut kl4_table = kernel_addrspace.as_page_table();
                let kl4_table = kl4_table.level_4_table();

                for i in 256..512 {
                    let i = PageTableIndex::new(i);
                    l4_table[i] = kl4_table[i].clone();
                }
            };

            addrspace
        }
    }

    fn as_page_table(&mut self) -> MappedPageTable<impl PageTableFrameMapping> {
        unsafe { MappedPageTable::new(&mut *(get_phys_mem_ptr_mut(self.0) as *mut PageTable), PhysPageTableFrameMapping) }
    }
}

pub(super) unsafe fn init_kernel_addrspace() {
    if (init_kernel_addrspace as *const () as u64) < 0xffff000000000000 {
        panic!("Kernel is loaded in lower-half?");
    };

    let mut kernel_addrspace = (*KERNEL_ADDRESS_SPACE.get()).lock();
    assert_eq!(kernel_addrspace.0, PhysAddr::zero());

    (*kernel_addrspace).0 = Cr3::read().0.start_address();

    let mut kl4_table = kernel_addrspace.as_page_table();

    {
        let mut frame_alloc = crate::frame_alloc::get_allocator().lock();

        // The bootloader will map some pages in the lower half of the address space, but these should no longer be used.
        // TODO: We can probably reclaim the frames used by these page tables
        for i in 0..256 {
            let i = PageTableIndex::new(i);
            kl4_table.level_4_table()[i].set_addr(PhysAddr::zero(), PageTableFlags::empty());
        }

        // All address spaces will have the same mappings for the upper entries. In order to allow pages in this area to be mapped into all
        // address spaces without needing to potentially update the L4 page tables of all address spaces, the L4 page table must have all of
        // these entries already filled.
        for i in 256..512 {
            let i = PageTableIndex::new(i);
            if kl4_table.level_4_table()[i].is_unused() {
                let kl3_table = frame_alloc.alloc_one().unwrap();
                ptr::write_bytes(get_phys_mem_ptr_mut(kl3_table) as *mut u8, 0, PAGE_SIZE);

                kl4_table.level_4_table()[i].set_addr(kl3_table, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            };
        }

        x86_64::instructions::tlb::flush_all();
    };
}
