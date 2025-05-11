//! Virtual memory region allocation.

use core::cmp::Ordering;
use core::{mem, ptr};

use static_assertions::const_assert;

use super::frame::{self, FrameAllocator};
use crate::arch::page::{get_phys_mem_ptr, PhysMemPtr, IS_PHYS_MEM_ALWAYS_MAPPED, PAGE_SIZE};
use crate::arch::VirtAddr;

#[derive(Debug, Clone, Copy)]
enum VirtualAllocSplitIndex {
    Prev(usize),
    This(usize),
    Next(usize),
}

/// A region of virtual memory as used by the allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualAllocRegion(VirtAddr, VirtAddr);

impl VirtualAllocRegion {
    /// Creates a new virtual memory region with the specified start and end addresses.
    ///
    /// # Panics
    ///
    /// This function will panic if `start > end`.
    pub fn new(start: VirtAddr, end: VirtAddr) -> VirtualAllocRegion {
        assert!(start <= end);
        VirtualAllocRegion(start, end)
    }

    /// Creates an arbitrary virtual memory region of size 0.
    pub const fn empty() -> VirtualAllocRegion {
        let addr = VirtAddr::new_truncate(PAGE_SIZE as u64);

        VirtualAllocRegion(addr, addr)
    }

    /// Gets the size of this virtual memory region in bytes.
    pub fn size(&self) -> u64 {
        self.1 - self.0
    }

    /// Gets the virtual address of the start of this virtual memory region.
    pub fn start(&self) -> VirtAddr {
        self.0
    }

    /// Gets the virtual address one byte past the end of this virtual memory region.
    pub fn end(&self) -> VirtAddr {
        self.1
    }

    /// Checks whether the start and end addresses of this virtual memory region are page-aligned.
    pub fn is_page_aligned(&self) -> bool {
        self.0.is_aligned(PAGE_SIZE as u64) && self.1.is_aligned(PAGE_SIZE as u64)
    }
}

struct VirtualAllocPageHeader {
    prev: *mut VirtualAllocPage,
    next: *mut VirtualAllocPage,
    len: u16,
}

impl VirtualAllocPageHeader {
    fn len(&self) -> usize {
        self.len as usize
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_full(&self) -> bool {
        self.len() == VirtualAllocPage::REGIONS_PER_PAGE
    }

    fn set_len(&mut self, len: usize) {
        self.len = u16::try_from(len).expect("Length of VirtualAllocPage too high");
    }
}

struct VirtualAllocPage {
    header: VirtualAllocPageHeader,
    regions: [VirtualAllocRegion; VirtualAllocPage::REGIONS_PER_PAGE],
}

impl VirtualAllocPage {
    const REGIONS_PER_PAGE: usize = (PAGE_SIZE - mem::size_of::<VirtualAllocPageHeader>()) / mem::size_of::<VirtualAllocRegion>();
    const SPLIT_POINT: usize = VirtualAllocPage::REGIONS_PER_PAGE / 2;
    const SPLIT_SECOND_SIZE: usize = VirtualAllocPage::REGIONS_PER_PAGE - VirtualAllocPage::SPLIT_POINT;

    fn new() -> *mut VirtualAllocPage {
        // TODO This currently relies on the fact that get_phys_mem_ptr does not require any
        //      virtual allocations to work, i.e. all of physical memory is mapped at all times
        const_assert!(IS_PHYS_MEM_ALWAYS_MAPPED);

        let ptr = get_phys_mem_ptr(frame::get_allocator().alloc_one().expect("Out of physical memory"));

        unsafe {
            *ptr.ptr() = VirtualAllocPage {
                header: VirtualAllocPageHeader {
                    prev: ptr::null_mut(),
                    next: ptr::null_mut(),
                    len: 0,
                },
                regions: [VirtualAllocRegion::empty(); VirtualAllocPage::REGIONS_PER_PAGE],
            };
        }

        ptr.into_raw()
    }

    unsafe fn free(ptr: *mut VirtualAllocPage) {
        let ptr = PhysMemPtr::from_raw(ptr);
        frame::get_allocator().free_one(ptr.phys_addr());
    }

    fn range(&self) -> Option<VirtualAllocRegion> {
        if self.header.is_empty() {
            None
        } else {
            Some(VirtualAllocRegion(self.regions[0].0, self.regions[self.header.len() - 1].1))
        }
    }

    fn split(&mut self, idx: usize) -> VirtualAllocSplitIndex {
        assert!(self.header.is_full());

        unsafe {
            if !self.header.prev.is_null() && (*self.header.prev).header.len() < VirtualAllocPage::REGIONS_PER_PAGE / 4 {
                let prev = &mut *self.header.prev;
                let idx = if idx < VirtualAllocPage::SPLIT_POINT {
                    VirtualAllocSplitIndex::Prev(prev.header.len() + idx)
                } else {
                    VirtualAllocSplitIndex::This(idx - VirtualAllocPage::SPLIT_POINT)
                };

                prev.regions[prev.header.len()..(prev.header.len() + VirtualAllocPage::SPLIT_POINT)]
                    .copy_from_slice(&self.regions[0..VirtualAllocPage::SPLIT_POINT]);
                self.regions.copy_within(VirtualAllocPage::SPLIT_POINT..self.header.len(), 0);

                prev.header.set_len(prev.header.len() + VirtualAllocPage::SPLIT_POINT);
                self.header.set_len(VirtualAllocPage::SPLIT_SECOND_SIZE);

                idx
            } else {
                let next = if !self.header.next.is_null() && (*self.header.next).header.len() < VirtualAllocPage::REGIONS_PER_PAGE / 4 {
                    &mut *self.header.next
                } else {
                    let next = &mut *VirtualAllocPage::new();

                    next.header.prev = self;
                    next.header.next = self.header.next;

                    self.header.next = next;

                    next
                };

                let idx = if idx >= VirtualAllocPage::SPLIT_POINT {
                    VirtualAllocSplitIndex::Next(idx - VirtualAllocPage::SPLIT_POINT)
                } else {
                    VirtualAllocSplitIndex::This(idx)
                };

                next.regions.copy_within(0..next.header.len(), VirtualAllocPage::SPLIT_SECOND_SIZE);
                next.regions[0..VirtualAllocPage::SPLIT_SECOND_SIZE]
                    .copy_from_slice(&self.regions[VirtualAllocPage::SPLIT_POINT..self.header.len()]);

                next.header.set_len(next.header.len() + VirtualAllocPage::SPLIT_SECOND_SIZE);
                self.header.set_len(VirtualAllocPage::SPLIT_POINT);

                idx
            }
        }
    }

    fn valid_regions(&self) -> &[VirtualAllocRegion] {
        &self.regions[0..self.header.len()]
    }

    fn find_idx_for_region_insert(&self, region: VirtualAllocRegion) -> Result<usize, usize> {
        let result = self.valid_regions().binary_search_by_key(&region.0, |r| r.1);

        let (idx_before, idx_after) = match result {
            Ok(idx) if idx == self.header.len() - 1 => (Some(idx), None),
            Ok(idx) => (Some(idx), Some(idx + 1)),
            Err(0) if self.header.is_empty() => (None, None),
            Err(0) => (None, Some(0)),
            Err(idx) if idx == self.header.len() => (Some(idx - 1), None),
            Err(idx) => (Some(idx - 1), Some(idx)),
        };

        let already_free = unsafe {
            let already_free_before = if let Some(idx_before) = idx_before {
                region.0 < self.regions[idx_before].1
            } else {
                !self.header.prev.is_null() && region.0 < (*self.header.prev).range().unwrap().1
            };

            let already_free_after = if let Some(idx_after) = idx_after {
                region.1 > self.regions[idx_after].0
            } else {
                !self.header.next.is_null() && region.1 > (*self.header.next).range().unwrap().0
            };

            already_free_before || already_free_after
        };

        if already_free {
            panic!("Attempt to free already free virtual region {:?}", region);
        }

        result
    }

    fn insert_region(&mut self, region: VirtualAllocRegion, idx: usize) {
        assert!(!self.header.is_full());

        self.regions.copy_within(idx..self.header.len(), idx + 1);
        self.regions[idx] = region;
        self.header.set_len(self.header.len() + 1);
    }

    fn remove_region(&mut self, idx: usize) {
        self.regions.copy_within((idx + 1)..self.header.len(), idx);
        self.header.set_len(self.header.len() - 1);
    }
}

/// An allocator that can be used to allocate regions of a virtual address space.
///
/// Note that unlike physical frame allocators, a virtual allocator is not permitted to make use of known free address space for its own
/// internal purposes. This allows these allocators to be used to allocate regions in a memory space that does not represent the currently
/// active address space, e.g. for processes other than the currently running process.
pub struct VirtualAllocator {
    head: *mut VirtualAllocPage,
}

impl VirtualAllocator {
    /// Creates a new empty virtual memory allocator.
    ///
    /// To populate the newly created allocator, [`VirtualAllocator::free`] should be called on regions known to be free in this address
    /// space.
    pub const fn new() -> VirtualAllocator {
        VirtualAllocator { head: ptr::null_mut() }
    }

    #[allow(clippy::if_same_then_else, clippy::needless_bool)]
    unsafe fn should_combine(p1: *mut VirtualAllocPage, p2: *mut VirtualAllocPage) -> bool {
        if (*p1).header.len() + (*p2).header.len() > VirtualAllocPage::REGIONS_PER_PAGE {
            false
        } else if (*p1).header.is_empty() || (*p2).header.is_empty() {
            true
        } else if (*p1).header.len() + (*p2).header.len() < VirtualAllocPage::REGIONS_PER_PAGE / 4 {
            true
        } else if (*p1).header.len().abs_diff((*p2).header.len()) >= VirtualAllocPage::REGIONS_PER_PAGE / 8 {
            true
        } else {
            false
        }
    }

    unsafe fn combine_pages(&mut self, p1: *mut VirtualAllocPage, p2: *mut VirtualAllocPage) {
        assert_eq!((*p1).header.next, p2);

        let p1 = &mut *p1;
        let p2 = &mut *p2;

        p1.regions[p1.header.len()..(p1.header.len() + p2.header.len())].copy_from_slice(&p2.regions[0..p2.header.len()]);
        p1.header.set_len(p1.header.len() + p2.header.len());

        p1.header.next = p2.header.next;
        if !p2.header.next.is_null() {
            (*p2.header.next).header.prev = p1;
        }

        VirtualAllocPage::free(p2);
    }

    unsafe fn combine_if_small(&mut self, page: *mut VirtualAllocPage) -> *mut VirtualAllocPage {
        let prev = (*page).header.prev;
        let next = (*page).header.next;

        if !prev.is_null() && VirtualAllocator::should_combine(prev, page) {
            self.combine_pages(prev, page);
            self.combine_if_small(prev)
        } else if !next.is_null() && VirtualAllocator::should_combine(page, next) {
            self.combine_pages(page, next);
            self.combine_if_small(page)
        } else {
            page
        }
    }

    unsafe fn try_coalesce_at(&mut self, page: *mut VirtualAllocPage) -> bool {
        let next = (*page).header.next;
        if (*page).header.is_empty() || next.is_null() {
            false
        } else if (*page).regions[(*page).header.len() - 1].1 == (*next).regions[0].0 {
            (*next).regions[0].0 = (*page).regions[(*page).header.len() - 1].0;
            (*page).header.set_len((*page).header.len() - 1);

            if (*next).header.len() == 1 {
                self.try_coalesce_at(next);
            }

            self.combine_if_small(page);
            true
        } else {
            false
        }
    }

    /// Frees the provided region of virtual memory in this address space.
    ///
    /// # Panics
    ///
    /// This function will panic if the starting address or size of the region provided is not aligned to the system's page size.
    ///
    /// # Safety
    ///
    /// While this function is not inherently unsafe in itself, it is unsafe when the address space in question is the kernel's own memory
    /// space. For such an allocator, it is necessary for the caller to ensure that the region in question really is free in order to avoid
    /// future kernel memory allocation requests returning virtual memory regions that are already in use.
    pub unsafe fn free(&mut self, region: VirtualAllocRegion) {
        assert!(region.is_page_aligned());

        if region.size() == 0 {
            return;
        }

        if self.head.is_null() {
            self.head = VirtualAllocPage::new();
        }

        let mut page = self.head;

        while !(*page).header.next.is_null() && region.1 >= (*(*page).header.next).range().unwrap().0 {
            page = (*page).header.next;
        }

        let page = &mut *page;
        let (page, idx): (*mut _, _) = match page.find_idx_for_region_insert(region) {
            //                +--------+
            //                | region |
            //                +--------+
            //                 VVVVVVVV
            // +--------------+--------+------------------+
            // | regions[idx] |        | regions[idx + 1] |
            // +--------------+--------+------------------+
            Ok(idx) if idx < page.header.len() - 1 && page.regions[idx + 1].0 == region.1 => {
                page.regions[idx].1 = page.regions[idx + 1].1;
                page.remove_region(idx + 1);
                (page, idx)
            },
            //                +--------+
            //                | region |
            //                +--------+
            //                 VVVVVVVV
            // +--------------+--------------+------------------+
            // | regions[idx] |              | regions[idx + 1] |
            // +--------------+--------------+------------------+
            Ok(idx) => {
                page.regions[idx].1 = region.1;
                (page, idx)
            },
            //                          +--------+
            //                          | region |
            //                          +--------+
            //                           VVVVVVVV
            // +------------------+--------------+--------------+
            // | regions[idx - 1] |              | regions[idx] |
            // +------------------+--------------+--------------+
            Err(idx) if idx < page.header.len() && page.regions[idx].0 == region.1 => {
                page.regions[idx].0 = region.0;
                (page, idx)
            },
            //                       +--------+
            //                       | region |
            //                       +--------+
            //                        VVVVVVVV
            // +------------------+--------------+--------------+
            // | regions[idx - 1] |              | regions[idx] |
            // +------------------+--------------+--------------+
            Err(idx) => {
                let (page, idx) = if page.header.is_full() {
                    match page.split(idx) {
                        VirtualAllocSplitIndex::Prev(idx) => (&mut *page.header.prev, idx),
                        VirtualAllocSplitIndex::This(idx) => (page, idx),
                        VirtualAllocSplitIndex::Next(idx) => (&mut *page.header.next, idx),
                    }
                } else {
                    (page, idx)
                };

                page.insert_region(region, idx);
                (page, idx)
            },
        };

        let coalesced = if idx == 0 && !(*page).header.prev.is_null() {
            self.try_coalesce_at((*page).header.prev)
        } else {
            false
        };

        if !coalesced && idx == (*page).header.len() - 1 {
            self.try_coalesce_at(page);
        }
    }

    /// Allocates a new region of virtual memory of `size` bytes in this address space. If no such region can be found, `None` is returned.
    ///
    /// # Panics
    ///
    /// This function will panic if the requested size is not a multiple of the systems page size.
    pub fn alloc(&mut self, size: usize) -> Option<VirtualAllocRegion> {
        assert_eq!(0, size & (PAGE_SIZE - 1));

        if size == 0 {
            return Some(VirtualAllocRegion::empty());
        }

        let mut page = self.head;
        while !page.is_null() {
            unsafe {
                for (idx, region) in (*page).valid_regions().iter().copied().enumerate().rev() {
                    match region.size().cmp(&(size as u64)) {
                        Ordering::Equal => {
                            (*page).remove_region(idx);
                            self.combine_if_small(page);
                            return Some(VirtualAllocRegion(region.0, region.0 + size));
                        },
                        Ordering::Greater => {
                            (*page).regions[idx].0 += size;
                            return Some(VirtualAllocRegion(region.0, region.0 + size));
                        },
                        Ordering::Less => {},
                    }
                }

                page = (*page).header.next;
            }
        }

        None
    }

    /// Removes the provided region of virtual memory from this virtual memory allocator if no part of it has already been allocated.
    /// Returns `true` on success. If one or more pages of the range passed in have already been allocated, then this function does not
    /// perform any modifications and returns `false`.
    ///
    /// # Panics
    ///
    /// This function will panic if the starting address or size of the region provided is not aligned to the system's page size.
    pub fn reserve(&mut self, region: VirtualAllocRegion) -> bool {
        assert!(region.is_page_aligned());
        if region.size() == 0 {
            return true;
        }

        let mut page = self.head;
        while !page.is_null() {
            unsafe {
                if let Some(range) = (*page).range() {
                    if region.start() < range.start() {
                        break;
                    } else if region.start() < range.end() {
                        let idx = match (*page).valid_regions().binary_search_by_key(&region.start(), |r| r.start()) {
                            Ok(idx) => idx,
                            Err(idx) => idx - 1,
                        };

                        let match_region = (*page).regions[idx];

                        if region.start() < match_region.end() && region.end() <= match_region.end() {
                            match (region.start() == match_region.start(), region.end() == match_region.end()) {
                                // +--------------+
                                // | region       |
                                // +--------------+
                                // | match_region |
                                // +--------------+
                                (true, true) => {
                                    (*page).remove_region(idx);
                                    self.combine_if_small(page);
                                },
                                // +--------+
                                // | region |
                                // +--------+-----+
                                // | match_region |
                                // +--------------+
                                (true, false) => {
                                    (*page).regions[idx].0 = region.end();
                                },
                                //       +--------+
                                //       | region |
                                // +-----+--------+
                                // | match_region |
                                // +--------------+
                                (false, true) => {
                                    (*page).regions[idx].1 = region.start();
                                },
                                //    +--------+
                                //    | region |
                                // +--+--------+--+
                                // | match_region |
                                // +--------------+
                                (false, false) => {
                                    let idx = if (*page).header.is_full() {
                                        match (*page).split(idx) {
                                            VirtualAllocSplitIndex::Prev(idx) => {
                                                page = (*page).header.prev;
                                                idx
                                            },
                                            VirtualAllocSplitIndex::This(idx) => idx,
                                            VirtualAllocSplitIndex::Next(idx) => {
                                                page = (*page).header.next;
                                                idx
                                            },
                                        }
                                    } else {
                                        idx
                                    };

                                    (*page).regions[idx].1 = region.start();
                                    (*page).insert_region(VirtualAllocRegion::new(region.end(), match_region.end()), idx + 1);
                                },
                            }

                            return true;
                        }

                        break;
                    }
                }

                page = (*page).header.next;
            };
        }

        false
    }

    /// Gets an iterator that returns currently free regions of virtual memory.
    ///
    /// Note that this iterator requires the address space to remain locked while it is being iterated. This is not normally a concern for
    /// user-space virtual allocators, but requires special care when operating on the kernel's virtual allocator since memory allocation
    /// will not be possible while this iterator exists. This means that seemingly innocuous uses of this method on the kernel's virtual
    /// allocator, such as collecting this iterator into a `Vec`, may deadlock.
    pub fn free_regions(&self) -> impl Iterator<Item = VirtualAllocRegion> + '_ {
        VirtualAllocatorRegionIter(
            unsafe {
                if self.head.is_null() || (*self.head).header.is_empty() {
                    None
                } else {
                    self.head.as_ref()
                }
            },
            0,
        )
    }
}

unsafe impl Send for VirtualAllocator {}

struct VirtualAllocatorRegionIter<'a>(Option<&'a VirtualAllocPage>, usize);

impl<'a> Iterator for VirtualAllocatorRegionIter<'a> {
    type Item = VirtualAllocRegion;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(page) = self.0 {
            let old_idx = self.1;

            self.1 = if old_idx == page.header.len() - 1 {
                self.0 = unsafe { page.header.next.as_ref() };
                0
            } else {
                old_idx + 1
            };

            Some(page.regions[old_idx])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use alloc::vec;
    use alloc::vec::Vec;

    use itertools::Itertools;

    use super::*;

    fn fake_region(page_idx: usize, num_pages: usize) -> VirtualAllocRegion {
        let start_addr = VirtAddr::new((PAGE_SIZE * (page_idx + 1)) as u64);

        VirtualAllocRegion::new(start_addr, start_addr + (num_pages * PAGE_SIZE))
    }

    #[test_case]
    fn test_basics() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 1));
            assert_eq!(vec![fake_region(0, 1)], allocator.free_regions().collect_vec());

            assert_eq!(Some(fake_region(0, 1)), allocator.alloc(PAGE_SIZE));
            assert_eq!(vec![] as Vec<VirtualAllocRegion>, allocator.free_regions().collect_vec());
            assert_eq!(None, allocator.alloc(PAGE_SIZE));
        };
    }

    #[test_case]
    fn test_region_order() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(4, 1));
            allocator.free(fake_region(0, 1));
            allocator.free(fake_region(2, 1));

            assert_eq!(
                vec![fake_region(0, 1), fake_region(2, 1), fake_region(4, 1)],
                allocator.free_regions().collect_vec()
            );
        }
    }

    #[test_case]
    fn test_alloc_region_sizes() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 3));
            allocator.free(fake_region(4, 2));
            allocator.free(fake_region(7, 1));

            assert_eq!(None, allocator.alloc(PAGE_SIZE * 4));
            assert_eq!(Some(fake_region(0, 3)), allocator.alloc(PAGE_SIZE * 3));
            assert_eq!(Some(fake_region(4, 2)), allocator.alloc(PAGE_SIZE * 2));
            assert_eq!(Some(fake_region(7, 1)), allocator.alloc(PAGE_SIZE));
        }
    }

    #[test_case]
    fn test_alloc_region_middle() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 1));
            allocator.free(fake_region(2, 2));
            allocator.free(fake_region(5, 1));

            assert_eq!(Some(fake_region(2, 2)), allocator.alloc(PAGE_SIZE * 2));
            assert_eq!(vec![fake_region(0, 1), fake_region(5, 1)], allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_alloc_split_region() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 2));

            assert_eq!(Some(fake_region(0, 1)), allocator.alloc(PAGE_SIZE));
            assert_eq!(Some(fake_region(1, 1)), allocator.alloc(PAGE_SIZE));
            assert_eq!(None, allocator.alloc(PAGE_SIZE));
        }
    }

    #[test_case]
    fn test_alloc_coalesce_region_end() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 1));
            allocator.free(fake_region(1, 1));

            assert_eq!(Some(fake_region(0, 2)), allocator.alloc(PAGE_SIZE * 2));
            assert_eq!(None, allocator.alloc(PAGE_SIZE));
        }
    }

    #[test_case]
    fn test_alloc_coalesce_region_start() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(1, 1));
            allocator.free(fake_region(0, 1));

            assert_eq!(Some(fake_region(0, 2)), allocator.alloc(PAGE_SIZE * 2));
            assert_eq!(None, allocator.alloc(PAGE_SIZE));
        }
    }

    #[test_case]
    fn test_alloc_coalesce_region_mid() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(2, 1));
            allocator.free(fake_region(0, 1));
            allocator.free(fake_region(1, 1));

            assert_eq!(Some(fake_region(0, 3)), allocator.alloc(PAGE_SIZE * 3));
            assert_eq!(None, allocator.alloc(PAGE_SIZE));
        }
    }

    #[test_case]
    fn test_reserve_region_full() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 1));
            allocator.free(fake_region(2, 1));
            allocator.free(fake_region(4, 1));

            assert!(allocator.reserve(fake_region(2, 1)));
            assert!(allocator.reserve(fake_region(4, 1)));
            assert!(allocator.reserve(fake_region(0, 1)));
            assert_eq!(vec![] as Vec<VirtualAllocRegion>, allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_reserve_region_at_start() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 3));

            assert!(allocator.reserve(fake_region(0, 1)));
            assert_eq!(vec![fake_region(1, 2)], allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_reserve_region_at_end() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 3));

            assert!(allocator.reserve(fake_region(2, 1)));
            assert_eq!(vec![fake_region(0, 2)], allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_reserve_region_in_middle() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(0, 3));

            assert!(allocator.reserve(fake_region(1, 1)));
            assert_eq!(vec![fake_region(0, 1), fake_region(2, 1)], allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_reserve_invalid() {
        unsafe {
            let mut allocator = VirtualAllocator::new();

            allocator.free(fake_region(1, 1));

            assert!(!allocator.reserve(fake_region(0, 1)));
            assert!(!allocator.reserve(fake_region(2, 1)));
            assert!(!allocator.reserve(fake_region(0, 2)));
            assert!(!allocator.reserve(fake_region(1, 2)));
            assert!(!allocator.reserve(fake_region(0, 3)));
            assert_eq!(vec![fake_region(1, 1)], allocator.free_regions().collect_vec());
        }
    }

    fn full_region_start(i: usize) -> usize {
        (i * 10) + 100
    }

    fn full_allocator_regions() -> impl Iterator<Item = VirtualAllocRegion> {
        (0..VirtualAllocPage::REGIONS_PER_PAGE).map(|i| fake_region(full_region_start(i), 3))
    }

    fn split_allocator_regions() -> impl Iterator<Item = VirtualAllocRegion> {
        (0..(VirtualAllocPage::REGIONS_PER_PAGE + 1)).map(|i| fake_region(full_region_start(i), 3))
    }

    unsafe fn create_full_allocator() -> VirtualAllocator {
        let mut allocator = VirtualAllocator::new();

        for r in full_allocator_regions() {
            allocator.free(r);
        }

        assert!(!allocator.head.is_null());
        assert!((*allocator.head).header.next.is_null());

        allocator
    }

    unsafe fn create_split_allocator() -> VirtualAllocator {
        let mut allocator = VirtualAllocator::new();

        for r in split_allocator_regions() {
            allocator.free(r);
        }

        assert!(!allocator.head.is_null());
        assert!(!(*allocator.head).header.next.is_null());
        assert_eq!(VirtualAllocPage::SPLIT_POINT, (*allocator.head).header.len());
        assert_eq!(
            VirtualAllocPage::SPLIT_SECOND_SIZE + 1,
            (*(*allocator.head).header.next).header.len()
        );

        assert_eq!(
            fake_region(full_region_start(VirtualAllocPage::SPLIT_POINT - 1), 3),
            (*allocator.head).regions[VirtualAllocPage::SPLIT_POINT - 1]
        );

        assert_eq!(
            fake_region(full_region_start(VirtualAllocPage::SPLIT_POINT), 3),
            (*(*allocator.head).header.next).regions[0]
        );

        allocator
    }

    #[test_case]
    fn test_split_page_before() {
        unsafe {
            let mut allocator = create_full_allocator();

            allocator.free(fake_region(0, 4));

            assert!(!allocator.head.is_null());
            assert_eq!((*allocator.head).header.len(), VirtualAllocPage::SPLIT_POINT + 1);
            assert!(!(*allocator.head).header.next.is_null());
            assert_eq!((*(*allocator.head).header.next).header.len(), VirtualAllocPage::SPLIT_SECOND_SIZE);

            assert_eq!(Some(fake_region(0, 4)), allocator.alloc(4 * PAGE_SIZE));
            assert_eq!(full_allocator_regions().collect_vec(), allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_split_page_after() {
        unsafe {
            let mut allocator = create_full_allocator();

            allocator.free(fake_region(full_region_start(VirtualAllocPage::REGIONS_PER_PAGE + 1), 4));

            assert!(!allocator.head.is_null());
            assert_eq!((*allocator.head).header.len(), VirtualAllocPage::SPLIT_POINT);
            assert!(!(*allocator.head).header.next.is_null());
            assert_eq!(
                (*(*allocator.head).header.next).header.len(),
                VirtualAllocPage::SPLIT_SECOND_SIZE + 1
            );

            assert_eq!(
                Some(fake_region(full_region_start(VirtualAllocPage::REGIONS_PER_PAGE + 1), 4)),
                allocator.alloc(4 * PAGE_SIZE)
            );
            assert_eq!(full_allocator_regions().collect_vec(), allocator.free_regions().collect_vec());
        }
    }

    #[test_case]
    fn test_coalesce_inter_page_region() {
        unsafe {
            let mut allocator = create_split_allocator();

            let coalescing_region_start = full_region_start(VirtualAllocPage::SPLIT_POINT - 1) + 3;
            let coalescing_region_end = full_region_start(VirtualAllocPage::SPLIT_POINT);

            allocator.free(fake_region(coalescing_region_start, 7));
            assert!(!allocator.head.is_null());
            assert!(!(*allocator.head).header.next.is_null());
            assert_eq!(VirtualAllocPage::SPLIT_POINT - 1, (*allocator.head).header.len());
            assert_eq!(
                VirtualAllocPage::SPLIT_SECOND_SIZE + 1,
                (*(*allocator.head).header.next).header.len()
            );

            assert_eq!(
                fake_region(coalescing_region_start - 3, coalescing_region_end - coalescing_region_start + 6),
                (*(*allocator.head).header.next).regions[0]
            );
        }
    }

    #[test_case]
    fn test_merge_pages_from_prev() {
        unsafe {
            let mut allocator = create_split_allocator();

            for r in split_allocator_regions().take(VirtualAllocPage::SPLIT_POINT) {
                assert!(allocator.reserve(r));
            }

            assert!(!allocator.head.is_null());
            assert!((*allocator.head).header.next.is_null());
            assert_eq!(
                split_allocator_regions().skip(VirtualAllocPage::SPLIT_POINT).collect_vec(),
                allocator.free_regions().collect_vec()
            );
        }
    }

    #[test_case]
    fn test_merge_pages_from_next() {
        unsafe {
            let mut allocator = create_split_allocator();

            for r in split_allocator_regions().skip(VirtualAllocPage::SPLIT_POINT) {
                assert!(allocator.reserve(r));
            }

            assert!(!allocator.head.is_null());
            assert!((*allocator.head).header.next.is_null());
            assert_eq!(
                split_allocator_regions().take(VirtualAllocPage::SPLIT_POINT).collect_vec(),
                allocator.free_regions().collect_vec()
            );
        }
    }
}
