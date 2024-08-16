use core::ops::Range;
use core::ptr;

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::PageTableFrameMapping;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{
    FrameDeallocator, MappedPageTable, Page, PageSize, PageTable, PageTableFlags, PageTableIndex, PhysFrame, Size1GiB, Size2MiB, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::mem::frame::{self, FrameAllocator};
use crate::mem::virt::{VirtualAllocRegion, VirtualAllocator};
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;
use crate::sync::UninterruptibleSpinlock;
use crate::util::{OneShotManualInit, SyncPtr};

pub const PAGE_SIZE: usize = 4096;
pub const IS_PHYS_MEM_ALWAYS_MAPPED: bool = true;

pub use crate::arch::api::page::PageFlags;

static PHYS_MEM_BASE: OneShotManualInit<SyncPtr<u8>> = OneShotManualInit::uninit();
static KERNEL_ADDRESS_SPACE: OneShotManualInit<UninterruptibleSpinlock<AddressSpace>> = OneShotManualInit::uninit();

pub fn init_phys_mem_base(phys_mem_base: *mut u8) {
    PHYS_MEM_BASE.set(SyncPtr::new(phys_mem_base));
}

pub fn get_phys_mem_base() -> *mut u8 {
    **PHYS_MEM_BASE.get()
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
    virtual_alloc: VirtualAllocator,
    is_kernel: bool,
}

impl AddressSpace {
    pub(super) const unsafe fn from_page_table(page_table: PhysAddr, is_kernel: bool) -> AddressSpace {
        AddressSpace {
            page_table,
            virtual_alloc: VirtualAllocator::new(),
            is_kernel,
        }
    }

    pub(crate) unsafe fn new_kernel() -> AddressSpace {
        AddressSpace::from_page_table(Cr3::read().0.start_address(), true)
    }

    pub fn kernel() -> UninterruptibleSpinlockGuard<'static, AddressSpace> {
        (*KERNEL_ADDRESS_SPACE.get()).lock()
    }

    pub fn new() -> AddressSpace {
        unsafe {
            let mut addrspace = AddressSpace::from_page_table(crate::mem::frame::get_allocator().alloc_one().unwrap(), false);
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
                VirtAddr::new(0x00007ffffffff000),
            ));

            addrspace
        }
    }

    pub(crate) unsafe fn init_kernel_virtual_alloc(&mut self) {
        fn find_free_regions_in(
            table: &PageTable,
            range: Range<usize>,
            start_addr: VirtAddr,
            level: u64,
            out: &mut VirtualAllocator,
            pending_region: &mut Option<VirtualAllocRegion>,
        ) {
            let page_size = PAGE_SIZE << ((level - 1) * 9);

            for (i, j) in range.enumerate() {
                let entry = &table[j];
                let start_addr = start_addr + (i * page_size);

                if entry.is_unused() {
                    match *pending_region {
                        Some(ref mut pending_region) if pending_region.end() == start_addr => {
                            *pending_region = VirtualAllocRegion::new(pending_region.start(), start_addr + page_size);
                        },
                        Some(ref mut pending_region) => {
                            unsafe {
                                out.free(*pending_region);
                            }

                            *pending_region = VirtualAllocRegion::new(start_addr, start_addr + page_size);
                        },
                        None => {
                            *pending_region = Some(VirtualAllocRegion::new(start_addr, start_addr + page_size));
                        },
                    }
                } else if level > 1 && !entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                    find_free_regions_in(
                        unsafe { &*get_phys_mem_ptr(entry.frame().unwrap().start_address()).ptr() },
                        0..512,
                        start_addr,
                        level - 1,
                        out,
                        pending_region,
                    );
                }
            }
        }

        let mut pending_region = None;

        find_free_regions_in(
            &*get_phys_mem_ptr(self.page_table).ptr(),
            256..511,
            VirtAddr::new(0xffff800000000000),
            4,
            &mut self.virtual_alloc,
            &mut pending_region,
        );

        if let Some(pending_region) = pending_region {
            unsafe {
                self.virtual_alloc.free(pending_region);
            }
        }
    }

    pub fn virtual_alloc(&mut self) -> &mut VirtualAllocator {
        &mut self.virtual_alloc
    }

    fn as_page_table(&mut self) -> MappedPageTable<impl PageTableFrameMapping> {
        unsafe {
            MappedPageTable::new(
                &mut *(get_phys_mem_ptr(self.page_table).ptr() as *mut PageTable),
                PhysPageTableFrameMapping,
            )
        }
    }

    fn to_generic_flags(in_flags: PageTableFlags) -> PageFlags {
        let mut out_flags = PageFlags::empty();

        if in_flags.contains(PageTableFlags::USER_ACCESSIBLE) {
            out_flags |= PageFlags::USER;
        }

        if in_flags.contains(PageTableFlags::WRITABLE) {
            out_flags |= PageFlags::WRITEABLE;
        }

        if !in_flags.contains(PageTableFlags::NO_EXECUTE) {
            out_flags |= PageFlags::EXECUTABLE;
        }

        out_flags
    }

    fn to_x86_64_flags(in_flags: PageFlags) -> PageTableFlags {
        let mut out_flags = PageTableFlags::PRESENT;

        if in_flags.contains(PageFlags::USER) {
            out_flags |= PageTableFlags::USER_ACCESSIBLE;
        }

        if in_flags.contains(PageFlags::WRITEABLE) {
            out_flags |= PageTableFlags::WRITABLE;
        }

        if !in_flags.contains(PageFlags::EXECUTABLE) {
            out_flags |= PageTableFlags::NO_EXECUTE;
        }

        out_flags
    }

    pub fn get_page(&self, addr: VirtAddr) -> Option<(PhysAddr, PageFlags)> {
        unsafe {
            let page = Page::<Size4KiB>::containing_address(addr);

            let l4_table = &*(get_phys_mem_ptr(self.page_table).ptr() as *mut PageTable);
            let l4_entry = &l4_table[page.p4_index()];
            if !l4_entry.flags().contains(PageTableFlags::PRESENT) {
                return None;
            }

            let l3_table = &*(get_phys_mem_ptr(l4_entry.addr()).ptr() as *mut PageTable);
            let l3_entry = &l3_table[page.p3_index()];
            if !l3_entry.flags().contains(PageTableFlags::PRESENT) {
                return None;
            }

            if l3_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                return Some((
                    l3_entry.addr() + (addr.as_u64() & (Size1GiB::SIZE - 1)),
                    Self::to_generic_flags(l3_entry.flags()),
                ));
            }

            let l2_table = &*(get_phys_mem_ptr(l3_entry.addr()).ptr() as *mut PageTable);
            let l2_entry = &l2_table[page.p2_index()];
            if !l2_entry.flags().contains(PageTableFlags::PRESENT) {
                return None;
            }

            if l2_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                return Some((
                    l2_entry.addr() + (addr.as_u64() & (Size2MiB::SIZE - 1)),
                    Self::to_generic_flags(l2_entry.flags()),
                ));
            }

            let l1_table = &*(get_phys_mem_ptr(l2_entry.addr()).ptr() as *mut PageTable);
            let l1_entry = &l1_table[page.p1_index()];
            if !l1_entry.flags().contains(PageTableFlags::PRESENT) {
                return None;
            }

            Some((
                l1_entry.addr() + (addr.as_u64() & (Size4KiB::SIZE - 1)),
                Self::to_generic_flags(l1_entry.flags()),
            ))
        }
    }

    #[track_caller]
    unsafe fn set_page_internal(&mut self, addr: VirtAddr, mapping: Option<(PhysAddr, PageFlags)>) {
        let page = Page::<Size4KiB>::from_start_address(addr).expect("bad address for page mapping");

        let l4_table = &mut *(get_phys_mem_ptr(self.page_table).ptr() as *mut PageTable);
        let l4_entry = &mut l4_table[page.p4_index()];
        if !l4_entry.flags().contains(PageTableFlags::PRESENT) {
            if mapping.is_some() {
                let new_l3_table = frame::get_allocator().alloc_one().expect("out of memory");
                *get_phys_mem_ptr(new_l3_table).ptr() = PageTable::new();

                l4_entry.set_addr(
                    new_l3_table,
                    PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE,
                );
            } else {
                return;
            }
        }

        let l3_table = &mut *(get_phys_mem_ptr(l4_entry.addr()).ptr() as *mut PageTable);
        let l3_entry = &mut l3_table[page.p3_index()];
        if !l3_entry.flags().contains(PageTableFlags::PRESENT) {
            if mapping.is_some() {
                let new_l2_table = frame::get_allocator().alloc_one().expect("out of memory");
                *get_phys_mem_ptr(new_l2_table).ptr() = PageTable::new();

                l3_entry.set_addr(
                    new_l2_table,
                    PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE,
                );
            } else {
                return;
            }
        }

        if l3_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
            unimplemented!();
        }

        let l2_table = &mut *(get_phys_mem_ptr(l3_entry.addr()).ptr() as *mut PageTable);
        let l2_entry = &mut l2_table[page.p2_index()];
        if !l2_entry.flags().contains(PageTableFlags::PRESENT) {
            if mapping.is_some() {
                let new_l1_table = frame::get_allocator().alloc_one().expect("out of memory");
                *get_phys_mem_ptr(new_l1_table).ptr() = PageTable::new();

                l2_entry.set_addr(
                    new_l1_table,
                    PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE,
                );
            } else {
                return;
            }
        }

        if l2_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
            unimplemented!();
        }

        let l1_table = &mut *(get_phys_mem_ptr(l2_entry.addr()).ptr() as *mut PageTable);
        let l1_entry = &mut l1_table[page.p1_index()];

        if let Some((frame, flags)) = mapping {
            l1_entry.set_addr(frame, Self::to_x86_64_flags(flags));
        } else {
            l1_entry.set_addr(PhysAddr::zero(), PageTableFlags::empty());
        }
    }

    #[track_caller]
    pub unsafe fn set_page_user(&mut self, addr: VirtAddr, mapping: Option<(PhysAddr, PageFlags)>) {
        if self.is_kernel {
            panic!("set_page_user cannot be called on the kernel address space");
        }

        if addr.as_u64() >= 0x0000_8000_0000_0000 {
            panic!("set_page_user can only be used on lower-half virtual addresses");
        }

        unsafe { self.set_page_internal(addr, mapping) };

        if Cr3::read().0.start_address() == self.page_table {
            // TODO Flush on other cores
            x86_64::instructions::tlb::flush(addr);
        }
    }

    #[track_caller]
    pub unsafe fn set_page_kernel(&mut self, addr: VirtAddr, mapping: Option<(PhysAddr, PageFlags)>) {
        if !self.is_kernel {
            panic!("set_page_kernel cannot be called on a user address space");
        }

        if addr.as_u64() < 0xffff_8000_0000_0000 {
            panic!("set_page_kernel can only be used on higher-half virtual addresses");
        }

        unsafe { self.set_page_internal(addr, mapping) };

        // TODO Flush on other cores
        x86_64::instructions::tlb::flush(addr);
    }
}

pub(super) unsafe fn init_kernel_addrspace() {
    if (init_kernel_addrspace as *const () as u64) < 0xffff_8000_0000_0000 {
        panic!("Kernel is loaded in lower-half?");
    };

    let mut kernel_addrspace = AddressSpace::new_kernel();
    kernel_addrspace.init_kernel_virtual_alloc();

    let mut kl4_table = kernel_addrspace.as_page_table();

    {
        let mut frame_alloc = crate::mem::frame::get_allocator().lock();

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
                    PAGE_SIZE,
                );

                kl4_table.level_4_table()[i].set_addr(kl3_table, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            };
        }

        KERNEL_ADDRESS_SPACE.set(UninterruptibleSpinlock::new(kernel_addrspace));
        x86_64::instructions::tlb::flush_all();
    }
}
