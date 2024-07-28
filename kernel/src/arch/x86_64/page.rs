use core::ops::Range;
use core::ptr;

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::PageTableFrameMapping;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{FrameDeallocator, MappedPageTable, PageTable, PageTableFlags, PageTableIndex, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

use crate::frame_alloc::FrameAllocator;
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;
use crate::sync::UninterruptibleSpinlock;
use crate::util::OneShotManualInit;
use crate::virtual_alloc::{VirtualAllocRegion, VirtualAllocator};

pub const PAGE_SIZE: usize = 4096;
pub const IS_PHYS_MEM_ALWAYS_MAPPED: bool = true;

static PHYS_MEM_BASE: OneShotManualInit<*mut u8> = OneShotManualInit::uninit();
static KERNEL_ADDRESS_SPACE: OneShotManualInit<UninterruptibleSpinlock<AddressSpace>> = OneShotManualInit::uninit();

pub fn init_phys_mem_base(phys_mem_base: *mut u8) {
    PHYS_MEM_BASE.set(phys_mem_base);
}

pub fn get_phys_mem_base() -> *mut u8 {
    *PHYS_MEM_BASE.get()
}

#[derive(Debug)]
pub struct PhysMemPtr<T: ?Sized>(*mut T);

impl<T: ?Sized> PhysMemPtr<T> {
    pub fn ptr(&self) -> *mut T {
        self.0
    }

    pub fn phys_addr(&self) -> PhysAddr {
        PhysAddr::new(self.0 as *const () as u64 - get_phys_mem_base() as u64)
    }

    pub fn into_raw(self) -> *mut T {
        self.0
    }

    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        PhysMemPtr(ptr)
    }
}

pub fn get_phys_mem_ptr<T>(phys_addr: PhysAddr) -> PhysMemPtr<T> {
    PhysMemPtr(get_phys_mem_base().wrapping_offset(phys_addr.as_u64() as isize) as *mut T)
}

pub fn get_phys_mem_ptr_slice<T>(phys_addr: PhysAddr, len: usize) -> PhysMemPtr<[T]> {
    PhysMemPtr(ptr::slice_from_raw_parts_mut(get_phys_mem_ptr::<T>(phys_addr).ptr(), len))
}

struct PhysPageTableFrameMapping;

unsafe impl PageTableFrameMapping for PhysPageTableFrameMapping {
    fn frame_to_pointer(&self, frame: PhysFrame) -> *mut PageTable {
        get_phys_mem_ptr(frame.start_address()).ptr()
    }
}

#[allow(dead_code)]
struct PhysFrameDeallocator<'a, T: FrameAllocator>(&'a mut T);

impl<'a, T: FrameAllocator> FrameDeallocator<Size4KiB> for PhysFrameDeallocator<'a, T> {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        self.0.free_one(frame.start_address())
    }
}

pub struct AddressSpace {
    page_table: PhysAddr,
    virtual_alloc: VirtualAllocator
}

impl AddressSpace {
    pub(super) const unsafe fn from_page_table(page_table: PhysAddr) -> AddressSpace {
        AddressSpace {
            page_table,
            virtual_alloc: VirtualAllocator::new()
        }
    }

    pub(crate) unsafe fn new_kernel() -> AddressSpace {
        AddressSpace::from_page_table(Cr3::read().0.start_address())
    }

    pub fn kernel() -> UninterruptibleSpinlockGuard<'static, AddressSpace> {
        (*KERNEL_ADDRESS_SPACE.get()).lock()
    }

    pub fn new() -> AddressSpace {
        unsafe {
            let mut addrspace = AddressSpace::from_page_table(crate::frame_alloc::get_allocator().alloc_one().unwrap());
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

            addrspace.virtual_alloc.free(VirtualAllocRegion::new(
                VirtAddr::new(PAGE_SIZE as u64),
                VirtAddr::new(0x00007ffffffff000)
            ));

            addrspace
        }
    }

    pub(crate) unsafe fn init_kernel_virtual_alloc(&mut self) {
        fn find_free_regions_in(table: &PageTable, range: Range<usize>, start_addr: VirtAddr, level: u64, out: &mut VirtualAllocator) {
            let page_size = PAGE_SIZE << ((level - 1) * 9);

            for (i, j) in range.enumerate() {
                let entry = &table[j];
                let start_addr = start_addr + (i * page_size);

                if entry.is_unused() {
                    unsafe {
                        out.free(VirtualAllocRegion::new(start_addr, start_addr + page_size));
                    }
                } else if level > 1 && !entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                    find_free_regions_in(
                        unsafe { &*get_phys_mem_ptr(entry.frame().unwrap().start_address()).ptr() },
                        0..512,
                        start_addr,
                        level - 1,
                        out
                    );
                }
            }
        }

        find_free_regions_in(
            &*get_phys_mem_ptr(self.page_table).ptr(),
            256..511,
            VirtAddr::new(0xffff800000000000),
            4,
            &mut self.virtual_alloc
        );
    }

    pub fn virtual_alloc(&mut self) -> &mut VirtualAllocator {
        &mut self.virtual_alloc
    }

    fn as_page_table(&mut self) -> MappedPageTable<impl PageTableFrameMapping> {
        unsafe {
            MappedPageTable::new(
                &mut *(get_phys_mem_ptr(self.page_table).ptr() as *mut PageTable),
                PhysPageTableFrameMapping
            )
        }
    }
}

pub(super) unsafe fn init_kernel_addrspace() {
    if (init_kernel_addrspace as *const () as u64) < 0xffff800000000000 {
        panic!("Kernel is loaded in lower-half?");
    };

    let mut kernel_addrspace = AddressSpace::new_kernel();
    kernel_addrspace.init_kernel_virtual_alloc();

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
                ptr::write_bytes(
                    get_phys_mem_ptr_slice(kl3_table, PAGE_SIZE).ptr().get_unchecked_mut(0) as *mut u8,
                    0,
                    PAGE_SIZE
                );

                kl4_table.level_4_table()[i].set_addr(kl3_table, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            };
        }

        KERNEL_ADDRESS_SPACE.set(UninterruptibleSpinlock::new(kernel_addrspace));
        x86_64::instructions::tlb::flush_all();
    }
}
