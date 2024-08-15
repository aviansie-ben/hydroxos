use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use super::PageBasedAlloc;
use crate::arch::page::PAGE_SIZE;
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;
use crate::sync::UninterruptibleSpinlock;
use crate::util::FixedBitVector;

struct SlabInfo {
    ptr: NonNull<()>,
    next: Option<NonNull<SlabInfo>>,
    next_free: Option<NonNull<SlabInfo>>,
    num_free: u16,
    free: FixedBitVector<512>,
}

impl SlabInfo {
    pub fn new(ptr: NonNull<()>, n: u16) -> SlabInfo {
        SlabInfo {
            ptr,
            next: None,
            next_free: None,
            num_free: n,
            free: FixedBitVector::new(true),
        }
    }

    pub unsafe fn get_obj(&self, idx: usize, size: usize) -> NonNull<()> {
        self.ptr.byte_add(idx * size)
    }
}

pub const fn pages_per_slab(size: usize) -> usize {
    assert!((size as usize) < PAGE_SIZE);

    match size {
        2048.. => 4,
        1024.. => 3,
        512..1024 => 2,
        _ => 1,
    }
}

pub struct SlabAllocInner<const SIZE: usize, const OWN_INFO: bool = false>
where
    [(); pages_per_slab(SIZE)]:,
{
    first: Option<NonNull<SlabInfo>>,
    first_free: Option<NonNull<SlabInfo>>,
}

unsafe impl<const SIZE: usize, const OWN_INFO: bool> Send for SlabAllocInner<SIZE, OWN_INFO> where [(); pages_per_slab(SIZE)]: {}

impl<const SIZE: usize, const OWN_INFO: bool> SlabAllocInner<SIZE, OWN_INFO>
where
    [(); pages_per_slab(SIZE)]:,
{
    const PAGES_PER_SLAB: usize = pages_per_slab(SIZE);
    const SLAB_SIZE: usize = Self::PAGES_PER_SLAB * PAGE_SIZE;
    const OBJECTS_PER_SLAB: usize = (Self::SLAB_SIZE / SIZE);

    pub const fn new() -> Self {
        if OWN_INFO {
            assert!(SIZE >= core::mem::size_of::<SlabInfo>());
        }

        Self {
            first: None,
            first_free: None,
        }
    }

    pub fn alloc(&mut self) -> Option<NonNull<[u8; SIZE]>> {
        let first_free = if let Some(first_free) = self.first_free {
            first_free
        } else {
            let slab_layout = Layout::from_size_align(SIZE, PAGE_SIZE).expect("bad PAGE_SIZE");
            let ptr = match PageBasedAlloc.allocate(slab_layout) {
                Ok(ptr) => ptr.cast(),
                Err(_) => {
                    return None;
                },
            };

            let slab = if OWN_INFO {
                let mut slab = SlabInfo::new(ptr, Self::OBJECTS_PER_SLAB as u16);
                slab.free.set(0, false);

                let slab_ptr = slab.ptr.cast();
                unsafe { slab_ptr.write(slab) };

                slab_ptr
            } else if let Some(slab_ptr) = SLAB_INFO.inner.lock().alloc() {
                let slab_ptr = slab_ptr.cast();

                unsafe {
                    slab_ptr.write(SlabInfo::new(ptr, Self::OBJECTS_PER_SLAB as u16));
                }
                slab_ptr
            } else {
                unsafe {
                    PageBasedAlloc.deallocate(ptr.cast(), slab_layout);
                }
                return None;
            };

            unsafe {
                (*slab.as_ptr()).next = self.first;
                (*slab.as_ptr()).next_free = self.first_free;
            }
            self.first = Some(slab);
            self.first_free = Some(slab);

            slab
        };

        let first_free = unsafe { &mut *first_free.as_ptr() };
        let idx = first_free.free.find_next(0).expect("slab in freelist has no free slots");

        first_free.free.set(idx, false);
        first_free.num_free -= 1;

        if first_free.num_free == 0 {
            self.first_free = first_free.next_free.take();
            first_free.next_free = None;
        }

        Some(unsafe { first_free.get_obj(idx, SIZE).cast() })
    }

    pub unsafe fn free(&mut self, ptr: NonNull<[u8; SIZE]>) {
        let ptr = ptr.cast();
        let mut next = self.first;

        while let Some(slab) = next {
            let slab = &mut *slab.as_ptr();
            if ptr >= slab.ptr && ptr < slab.ptr.byte_add(Self::SLAB_SIZE) {
                let slab_off = ptr.byte_offset_from(slab.ptr) as usize;
                let idx = slab_off / SIZE;

                if slab_off != idx * SIZE {
                    panic!("attempt to free misaligned pointer");
                }

                if slab.free.set(idx, true) {
                    panic!("double free detected");
                }

                slab.num_free += 1;
                if slab.num_free == 1 {
                    slab.next_free = self.first_free;
                    self.first_free = Some(NonNull::from(slab));
                }

                return;
            }

            next = slab.next;
        }

        panic!("attempt to free pointer in the wrong slab allocator");
    }

    pub fn count(&self) -> (usize, usize) {
        let mut total = 0;
        let mut free = 0;

        let mut next = self.first;

        while let Some(slab) = next {
            let slab = unsafe { &mut *slab.as_ptr() };

            total += Self::OBJECTS_PER_SLAB;
            free += slab.num_free as usize;

            next = slab.next;
        }

        (total - free, total)
    }
}

impl<const SIZE: usize, const OWN_INFO: bool> Drop for SlabAllocInner<SIZE, OWN_INFO>
where
    [(); pages_per_slab(SIZE)]:,
{
    fn drop(&mut self) {
        let mut next_info = self.first;

        while let Some(mut slab) = next_info {
            let slab = unsafe { slab.as_mut() };

            if slab.num_free as usize != slab.free.count() {
                panic!("inconsistent # of free slots in slab");
            } else if slab.num_free as usize != Self::OBJECTS_PER_SLAB {
                panic!("slab freed while objects still allocated");
            }

            // NOTE: Must read these *before* we deallocate anything, since slab will be inside
            //       these pages when OWN_INFO is true.
            let ptr = slab.ptr;
            next_info = slab.next;

            unsafe {
                PageBasedAlloc.deallocate(
                    ptr.cast(),
                    Layout::from_size_align(Self::PAGES_PER_SLAB * PAGE_SIZE, PAGE_SIZE).unwrap(),
                );
            }

            if !OWN_INFO {
                unsafe {
                    SLAB_INFO.inner.lock().free(NonNull::from(slab).cast());
                }
            }
        }
    }
}

pub struct SlabAlloc<const SIZE: usize, const OWN_INFO: bool = false>
where
    [(); pages_per_slab(SIZE)]:,
{
    name: &'static str,
    inner: UninterruptibleSpinlock<SlabAllocInner<SIZE, OWN_INFO>>,
}

impl<const SIZE: usize, const OWN_INFO: bool> SlabAlloc<SIZE, OWN_INFO>
where
    [(); pages_per_slab(SIZE)]:,
{
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            inner: UninterruptibleSpinlock::new(SlabAllocInner::new()),
        }
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn lock(&self) -> UninterruptibleSpinlockGuard<SlabAllocInner<SIZE, OWN_INFO>> {
        self.inner.lock()
    }
}

unsafe impl<const SIZE: usize, const OWN_INFO: bool> Allocator for SlabAlloc<SIZE, OWN_INFO>
where
    [(); pages_per_slab(SIZE)]:,
{
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() > SIZE || SIZE.next_multiple_of(layout.align()) != SIZE {
            return Err(AllocError);
        }

        match self.inner.lock().alloc() {
            Some(ptr) => Ok(ptr),
            None => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        self.inner.lock().free(ptr.cast())
    }

    unsafe fn grow(&self, ptr: NonNull<u8>, _old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if new_layout.size() > SIZE || SIZE.next_multiple_of(new_layout.align()) != SIZE {
            return Err(AllocError);
        }

        Ok(NonNull::from_raw_parts(ptr.cast(), SIZE))
    }

    unsafe fn shrink(&self, ptr: NonNull<u8>, _old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if SIZE.next_multiple_of(new_layout.align()) != SIZE {
            return Err(AllocError);
        }

        Ok(NonNull::from_raw_parts(ptr.cast(), SIZE))
    }
}

pub static SLAB_INFO: SlabAlloc<{ core::mem::size_of::<SlabInfo>() }, true> = SlabAlloc::new("SLAB_INFO");

pub static SLAB_8: SlabAlloc<8> = SlabAlloc::new("SLAB_8");
pub static SLAB_16: SlabAlloc<16> = SlabAlloc::new("SLAB_16");
pub static SLAB_32: SlabAlloc<32> = SlabAlloc::new("SLAB_32");
pub static SLAB_64: SlabAlloc<64> = SlabAlloc::new("SLAB_64");
pub static SLAB_128: SlabAlloc<128> = SlabAlloc::new("SLAB_128");
pub static SLAB_256: SlabAlloc<256> = SlabAlloc::new("SLAB_256");
pub static SLAB_512: SlabAlloc<512> = SlabAlloc::new("SLAB_512");
pub static SLAB_1024: SlabAlloc<1024> = SlabAlloc::new("SLAB_1024");
pub static SLAB_2048: SlabAlloc<2048> = SlabAlloc::new("SLAB_2048");

#[cfg(test)]
mod test {
    use super::*;
    use crate::arch::page::AddressSpace;
    use crate::arch::VirtAddr;

    fn create_alloc<const N: usize, const OWN_INFO: bool>() -> SlabAlloc<N, OWN_INFO>
    where
        [(); pages_per_slab(N)]:,
    {
        SlabAlloc::new("TEST")
    }

    #[test_case]
    fn test_single_alloc_free() {
        let alloc = create_alloc::<8, false>();

        {
            let alloc = alloc.inner.lock();

            assert_eq!(alloc.first, None);
            assert_eq!(alloc.first_free, None);
            assert_eq!(alloc.count(), (0, 0));
        }

        let ptr = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(0));
            assert_eq!(ptr.cast(), slab_info.ptr);

            assert_eq!(alloc.count(), (1, SlabAllocInner::<8>::OBJECTS_PER_SLAB));

            for i in 0..SlabAllocInner::<8>::PAGES_PER_SLAB {
                assert!(!AddressSpace::kernel()
                    .get_page(VirtAddr::from_ptr(ptr.as_ptr()) + i * PAGE_SIZE)
                    .is_none());
            }
        }

        unsafe {
            alloc.deallocate(ptr.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB);

            assert_eq!(alloc.count(), (0, SlabAllocInner::<8>::OBJECTS_PER_SLAB));
        }
    }

    #[test_case]
    fn test_multi_alloc_free() {
        let alloc = create_alloc::<8, false>();

        let ptr_a = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");
        let ptr_b = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 2);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 2);
            assert!(!slab_info.free.get(0));
            assert!(!slab_info.free.get(1));
            assert_eq!(ptr_a.cast(), slab_info.ptr);
            assert_eq!(ptr_b.cast(), unsafe { slab_info.ptr.byte_add(8) });

            assert_eq!(alloc.count(), (2, SlabAllocInner::<8>::OBJECTS_PER_SLAB));
        }

        unsafe {
            alloc.deallocate(ptr_a.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(1));

            assert_eq!(alloc.count(), (1, SlabAllocInner::<8>::OBJECTS_PER_SLAB));
        }

        let ptr_c = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");
        assert_eq!(ptr_c, ptr_a);

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 2);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 2);
            assert!(!slab_info.free.get(0));
            assert!(!slab_info.free.get(1));

            assert_eq!(alloc.count(), (2, SlabAllocInner::<8>::OBJECTS_PER_SLAB));
        }

        unsafe {
            alloc.deallocate(ptr_b.cast(), Layout::new::<u64>());
            alloc.deallocate(ptr_c.cast(), Layout::new::<u64>());
        }
    }

    #[test_case]
    fn test_alloc_free_full_slab() {
        let alloc = create_alloc::<8, false>();
        let ptr_a = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        for i in 1..SlabAllocInner::<8>::OBJECTS_PER_SLAB {
            assert_eq!(Ok(unsafe { ptr_a.byte_add(i * 8) }), alloc.allocate(Layout::new::<u64>()));
        }

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.first_free, None);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, 0);
            assert_eq!(slab_info.free.count(), 0);
            assert_eq!(ptr_a.cast(), slab_info.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAllocInner::<8>::OBJECTS_PER_SLAB, SlabAllocInner::<8>::OBJECTS_PER_SLAB)
            );
        }

        let ptr_b = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have 2 slabs").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(0));
            assert_eq!(ptr_b.cast(), slab_info.ptr);

            let slab_info_2 = unsafe { &*slab_info.next.expect("should have 2 slabs").as_ptr() };

            assert_eq!(slab_info_2.next, None);
            assert_eq!(slab_info_2.next_free, None);

            assert_eq!(slab_info_2.num_free as usize, 0);
            assert_eq!(slab_info_2.free.count(), 0);
            assert_eq!(ptr_a.cast(), slab_info_2.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAllocInner::<8>::OBJECTS_PER_SLAB + 1, SlabAllocInner::<8>::OBJECTS_PER_SLAB * 2)
            );
        }

        unsafe {
            alloc.deallocate(ptr_a.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have 2 slabs").as_ptr() };

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(0));
            assert_eq!(ptr_b.cast(), slab_info.ptr);

            let slab_info_2 = unsafe { &*slab_info.next.expect("should have 2 slabs").as_ptr() };

            assert_eq!(alloc.first_free, Some(NonNull::from(slab_info_2)));
            assert_eq!(slab_info_2.next, None);
            assert_eq!(slab_info_2.next_free, Some(NonNull::from(slab_info)));

            assert_eq!(slab_info_2.num_free as usize, 1);
            assert_eq!(slab_info_2.free.count(), 1);
            assert!(slab_info_2.free.get(0));
            assert_eq!(ptr_a.cast(), slab_info_2.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAllocInner::<8>::OBJECTS_PER_SLAB, SlabAllocInner::<8>::OBJECTS_PER_SLAB * 2)
            );
        }

        let ptr_c = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");
        assert_eq!(ptr_c, ptr_a);

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have 2 slabs").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(0));
            assert_eq!(ptr_b.cast(), slab_info.ptr);

            let slab_info_2 = unsafe { &*slab_info.next.expect("should have 2 slabs").as_ptr() };

            assert_eq!(slab_info_2.next, None);
            assert_eq!(slab_info_2.next_free, None);

            assert_eq!(slab_info_2.num_free as usize, 0);
            assert_eq!(slab_info_2.free.count(), 0);
            assert_eq!(ptr_a.cast(), slab_info_2.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAllocInner::<8>::OBJECTS_PER_SLAB + 1, SlabAllocInner::<8>::OBJECTS_PER_SLAB * 2)
            );
        }

        unsafe {
            alloc.deallocate(ptr_b.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.inner.lock();

            let slab_info = unsafe { &*alloc.first.expect("should have 2 slabs").as_ptr() };
            assert_eq!(alloc.first_free, alloc.first);

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAllocInner::<8>::OBJECTS_PER_SLAB);
            assert_eq!(slab_info.free.count(), SlabAllocInner::<8>::OBJECTS_PER_SLAB);
            assert_eq!(ptr_b.cast(), slab_info.ptr);

            let slab_info_2 = unsafe { &*slab_info.next.expect("should have 2 slabs").as_ptr() };

            assert_eq!(slab_info_2.next, None);
            assert_eq!(slab_info_2.next_free, None);

            assert_eq!(slab_info_2.num_free as usize, 0);
            assert_eq!(slab_info_2.free.count(), 0);
            assert_eq!(ptr_a.cast(), slab_info_2.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAllocInner::<8>::OBJECTS_PER_SLAB, SlabAllocInner::<8>::OBJECTS_PER_SLAB * 2)
            );
        }

        unsafe {
            for i in 0..SlabAllocInner::<8>::OBJECTS_PER_SLAB {
                alloc.deallocate(ptr_a.byte_add(i * 8).cast(), Layout::new::<u64>());
            }
        }
    }
}
