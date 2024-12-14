use core::alloc::{AllocError, Allocator, Layout};
use core::cell::SyncUnsafeCell;
use core::marker::PhantomData;
use core::mem;
use core::ptr::NonNull;

use super::PageBasedAlloc;
use crate::arch::page::PAGE_SIZE;
use crate::sync::uninterruptible::UninterruptibleSpinlockGuard;
use crate::sync::UninterruptibleSpinlock;
use crate::util::FixedBitVector;

pub struct SlabInfo {
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
            free: {
                let mut free = FixedBitVector::new(false);
                free.set_range(..(n as usize), true);
                free
            },
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

struct SlabList {
    first: Option<NonNull<SlabInfo>>,
    first_free: Option<NonNull<SlabInfo>>,
}

unsafe impl Send for SlabList {}

impl SlabList {
    const fn new() -> Self {
        Self {
            first: None,
            first_free: None,
        }
    }
}

struct SlabAllocListInfo {
    registered: bool,
    next: Option<NonNull<SlabAllocAny>>,
}

unsafe impl Send for SlabAllocListInfo {}
unsafe impl Sync for SlabAllocListInfo {}

pub struct SlabAllocAny {
    name: &'static str,
    obj_size: usize,
    list_info: SyncUnsafeCell<SlabAllocListInfo>,
    slabs: UninterruptibleSpinlock<SlabList>,
}

impl SlabAllocAny {
    const fn new(name: &'static str, obj_size: usize) -> Self {
        Self {
            name,
            obj_size,
            list_info: SyncUnsafeCell::new(SlabAllocListInfo {
                registered: false,
                next: None,
            }),
            slabs: UninterruptibleSpinlock::new(SlabList::new()),
        }
    }

    pub fn object_size(&self) -> usize {
        self.obj_size
    }

    pub fn objects_per_slab(&self) -> usize {
        (pages_per_slab(self.obj_size) * PAGE_SIZE) / self.obj_size
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn lock(&self) -> SlabAllocAnyLock {
        SlabAllocAnyLock {
            alloc: self,
            slabs: self.slabs.lock(),
        }
    }

    fn count(&self, slabs: &UninterruptibleSpinlockGuard<SlabList>) -> (usize, usize) {
        let mut total = 0;
        let mut free = 0;

        let mut next = slabs.first;

        while let Some(slab) = next {
            let slab = unsafe { &mut *slab.as_ptr() };

            total += self.objects_per_slab();
            free += slab.num_free as usize;

            next = slab.next;
        }

        (total - free, total)
    }
}

pub struct SlabAllocAnyLock<'a> {
    alloc: &'a SlabAllocAny,
    slabs: UninterruptibleSpinlockGuard<'a, SlabList>,
}

impl<'a> SlabAllocAnyLock<'a> {
    pub fn slab_alloc(&self) -> &'a SlabAllocAny {
        self.alloc
    }

    pub fn count(&self) -> (usize, usize) {
        self.alloc.count(&self.slabs)
    }
}

pub struct SlabAlloc<T, const OWN_INFO: bool = false> {
    inner: SlabAllocAny,
    _data: PhantomData<fn(T) -> T>,
}

impl<T, const OWN_INFO: bool> SlabAlloc<T, OWN_INFO> {
    const OBJECT_SIZE: usize = mem::size_of::<T>();

    const PAGES_PER_SLAB: usize = pages_per_slab(Self::OBJECT_SIZE);
    const SLAB_SIZE: usize = Self::PAGES_PER_SLAB * PAGE_SIZE;
    const OBJECTS_PER_SLAB: usize = (Self::SLAB_SIZE / Self::OBJECT_SIZE);

    pub const fn new(name: &'static str) -> Self {
        if OWN_INFO {
            assert!(Self::OBJECT_SIZE >= mem::size_of::<SlabInfo>());
            assert!(Self::OBJECT_SIZE % mem::align_of::<SlabInfo>() == 0);
        }

        Self {
            inner: SlabAllocAny::new(name, Self::OBJECT_SIZE),
            _data: PhantomData,
        }
    }

    pub fn register(&'static self) {
        let mut guard = SLAB_ALLOCS.lock();
        let list_info = unsafe { &mut *self.inner.list_info.get() };

        assert!(!list_info.registered);
        list_info.registered = true;

        if let Some(last) = guard.last {
            unsafe {
                (*(*last.as_ptr()).list_info.get()).next = Some(NonNull::from(&self.inner));
            }

            guard.last = Some(NonNull::from(&self.inner));
        } else {
            guard.first = Some(NonNull::from(&self.inner));
            guard.last = Some(NonNull::from(&self.inner));
        }
    }

    pub fn as_any(&self) -> &SlabAllocAny {
        &self.inner
    }

    pub fn name(&self) -> &str {
        self.inner.name()
    }

    pub fn lock(&self) -> SlabAllocLock<T, OWN_INFO> {
        SlabAllocLock {
            alloc: self,
            slabs: self.inner.slabs.lock(),
        }
    }
}

unsafe impl<T, const OWN_INFO: bool> Allocator for SlabAlloc<T, OWN_INFO> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() > Self::OBJECT_SIZE || Self::OBJECT_SIZE.next_multiple_of(layout.align()) != Self::OBJECT_SIZE {
            return Err(AllocError);
        }

        match self.lock().alloc() {
            Some(ptr) => Ok(NonNull::from_raw_parts(ptr, Self::OBJECT_SIZE)),
            None => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        self.lock().free(ptr.cast())
    }

    unsafe fn grow(&self, ptr: NonNull<u8>, _old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if new_layout.size() > Self::OBJECT_SIZE || Self::OBJECT_SIZE.next_multiple_of(new_layout.align()) != Self::OBJECT_SIZE {
            return Err(AllocError);
        }

        Ok(NonNull::from_raw_parts(ptr, Self::OBJECT_SIZE))
    }

    unsafe fn shrink(&self, ptr: NonNull<u8>, _old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if Self::OBJECT_SIZE.next_multiple_of(new_layout.align()) != Self::OBJECT_SIZE {
            return Err(AllocError);
        }

        Ok(NonNull::from_raw_parts(ptr, Self::OBJECT_SIZE))
    }
}

impl<T, const OWN_INFO: bool> Drop for SlabAlloc<T, OWN_INFO> {
    fn drop(&mut self) {
        let mut next_info = self.inner.slabs.get_mut().first;

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
                    SLAB_INFO.lock().free(NonNull::from(slab).cast());
                }
            }
        }
    }
}

pub struct SlabAllocLock<'a, T, const OWN_INFO: bool> {
    alloc: &'a SlabAlloc<T, OWN_INFO>,
    slabs: UninterruptibleSpinlockGuard<'a, SlabList>,
}

impl<'a, T, const OWN_INFO: bool> SlabAllocLock<'a, T, OWN_INFO> {
    pub fn slab_alloc(&self) -> &'a SlabAlloc<T, OWN_INFO> {
        self.alloc
    }

    pub fn into_any(self) -> SlabAllocAnyLock<'a> {
        SlabAllocAnyLock {
            alloc: &self.alloc.inner,
            slabs: self.slabs,
        }
    }

    pub fn alloc(&mut self) -> Option<NonNull<T>> {
        let first_free = if let Some(first_free) = self.slabs.first_free {
            first_free
        } else {
            let slab_layout = Layout::from_size_align(SlabAlloc::<T, OWN_INFO>::SLAB_SIZE, PAGE_SIZE).expect("bad PAGE_SIZE");
            let ptr = match PageBasedAlloc.allocate(slab_layout) {
                Ok(ptr) => ptr.cast(),
                Err(_) => {
                    return None;
                },
            };

            let slab = if OWN_INFO {
                let mut slab = SlabInfo::new(ptr, SlabAlloc::<T, OWN_INFO>::OBJECTS_PER_SLAB as u16);
                slab.free.set(0, false);
                slab.num_free -= 1;

                let slab_ptr = slab.ptr.cast();
                unsafe { slab_ptr.write(slab) };

                slab_ptr
            } else if let Some(slab_ptr) = SLAB_INFO.lock().alloc() {
                unsafe {
                    slab_ptr.write(SlabInfo::new(ptr, SlabAlloc::<T, OWN_INFO>::OBJECTS_PER_SLAB as u16));
                }
                slab_ptr
            } else {
                unsafe {
                    PageBasedAlloc.deallocate(ptr.cast(), slab_layout);
                }
                return None;
            };

            unsafe {
                (*slab.as_ptr()).next = self.slabs.first;
                (*slab.as_ptr()).next_free = self.slabs.first_free;
            }
            self.slabs.first = Some(slab);
            self.slabs.first_free = Some(slab);

            slab
        };

        let first_free = unsafe { &mut *first_free.as_ptr() };
        let idx = first_free.free.find_next(0).expect("slab in freelist has no free slots");

        first_free.free.set(idx, false);
        first_free.num_free -= 1;

        if first_free.num_free == 0 {
            self.slabs.first_free = first_free.next_free.take();
            first_free.next_free = None;
        }

        Some(unsafe { first_free.get_obj(idx, SlabAlloc::<T, OWN_INFO>::OBJECT_SIZE).cast() })
    }

    pub unsafe fn free(&mut self, ptr: NonNull<T>) {
        let ptr = ptr.cast();
        let mut next = self.slabs.first;

        while let Some(slab) = next {
            let slab = &mut *slab.as_ptr();
            if ptr >= slab.ptr && ptr < slab.ptr.byte_add(SlabAlloc::<T, OWN_INFO>::SLAB_SIZE) {
                let slab_off = ptr.byte_offset_from(slab.ptr) as usize;
                let idx = slab_off / SlabAlloc::<T, OWN_INFO>::OBJECT_SIZE;

                if slab_off != idx * SlabAlloc::<T, OWN_INFO>::OBJECT_SIZE {
                    panic!("attempt to free misaligned pointer");
                }

                if slab.free.set(idx, true) {
                    panic!("double free detected");
                }

                slab.num_free += 1;
                if slab.num_free == 1 {
                    slab.next_free = self.slabs.first_free;
                    self.slabs.first_free = Some(NonNull::from(slab));
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

        let mut next = self.slabs.first;

        while let Some(slab) = next {
            let slab = unsafe { &mut *slab.as_ptr() };

            total += SlabAlloc::<T, OWN_INFO>::OBJECTS_PER_SLAB;
            free += slab.num_free as usize;

            next = slab.next;
        }

        (total - free, total)
    }
}

struct SlabAllocList {
    first: Option<NonNull<SlabAllocAny>>,
    last: Option<NonNull<SlabAllocAny>>,
}

unsafe impl Send for SlabAllocList {}

pub struct SlabAllocListIter {
    next: Option<NonNull<SlabAllocAny>>,
    _guard: UninterruptibleSpinlockGuard<'static, SlabAllocList>,
}

impl Iterator for SlabAllocListIter {
    type Item = &'static SlabAllocAny;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(alloc) = self.next {
            let alloc = unsafe { &*alloc.as_ptr() };
            self.next = unsafe { (*alloc.list_info.get()).next };

            Some(alloc)
        } else {
            None
        }
    }
}

static SLAB_ALLOCS: UninterruptibleSpinlock<SlabAllocList> = UninterruptibleSpinlock::new(SlabAllocList { first: None, last: None });

static SLAB_INFO: SlabAlloc<SlabInfo, true> = SlabAlloc::new("SLAB_INFO");

pub static SLAB_8: SlabAlloc<[u8; 8]> = SlabAlloc::new("SLAB_8");
pub static SLAB_16: SlabAlloc<[u8; 16]> = SlabAlloc::new("SLAB_16");
pub static SLAB_32: SlabAlloc<[u8; 32]> = SlabAlloc::new("SLAB_32");
pub static SLAB_64: SlabAlloc<[u8; 64]> = SlabAlloc::new("SLAB_64");
pub static SLAB_128: SlabAlloc<[u8; 128]> = SlabAlloc::new("SLAB_128");
pub static SLAB_256: SlabAlloc<[u8; 256]> = SlabAlloc::new("SLAB_256");
pub static SLAB_512: SlabAlloc<[u8; 512]> = SlabAlloc::new("SLAB_512");
pub static SLAB_1024: SlabAlloc<[u8; 1024]> = SlabAlloc::new("SLAB_1024");
pub static SLAB_2048: SlabAlloc<[u8; 2048]> = SlabAlloc::new("SLAB_2048");

pub fn registered_slab_allocs() -> SlabAllocListIter {
    let guard = SLAB_ALLOCS.lock();

    SlabAllocListIter {
        next: guard.first,
        _guard: guard,
    }
}

pub(super) fn init() {
    SLAB_INFO.register();
    SLAB_8.register();
    SLAB_16.register();
    SLAB_32.register();
    SLAB_64.register();
    SLAB_128.register();
    SLAB_256.register();
    SLAB_512.register();
    SLAB_1024.register();
    SLAB_2048.register();
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::arch::page::AddressSpace;
    use crate::arch::VirtAddr;

    fn create_alloc<const N: usize, const OWN_INFO: bool>() -> SlabAlloc<[u8; N], OWN_INFO>
    where
        [(); N]:,
    {
        SlabAlloc::new("TEST")
    }

    #[test_case]
    fn test_single_alloc_free() {
        let alloc = create_alloc::<8, false>();

        {
            let alloc = alloc.lock();

            assert_eq!(alloc.slabs.first, None);
            assert_eq!(alloc.slabs.first_free, None);
            assert_eq!(alloc.count(), (0, 0));
        }

        let ptr = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(0));
            assert_eq!(ptr.cast(), slab_info.ptr);

            assert_eq!(alloc.count(), (1, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB));

            for i in 0..SlabAlloc::<[u8; 8]>::PAGES_PER_SLAB {
                assert!(!AddressSpace::kernel()
                    .get_page(VirtAddr::from_ptr(ptr.as_ptr()) + i * PAGE_SIZE)
                    .is_none());
            }
        }

        unsafe {
            alloc.deallocate(ptr.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB);

            assert_eq!(alloc.count(), (0, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB));
        }
    }

    #[test_case]
    fn test_multi_alloc_free() {
        let alloc = create_alloc::<8, false>();

        let ptr_a = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");
        let ptr_b = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 2);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 2);
            assert!(!slab_info.free.get(0));
            assert!(!slab_info.free.get(1));
            assert_eq!(ptr_a.cast(), slab_info.ptr);
            assert_eq!(ptr_b.cast(), unsafe { slab_info.ptr.byte_add(8) });

            assert_eq!(alloc.count(), (2, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB));
        }

        unsafe {
            alloc.deallocate(ptr_a.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(1));

            assert_eq!(alloc.count(), (1, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB));
        }

        let ptr_c = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");
        assert_eq!(ptr_c, ptr_a);

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 2);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 2);
            assert!(!slab_info.free.get(0));
            assert!(!slab_info.free.get(1));

            assert_eq!(alloc.count(), (2, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB));
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

        for i in 1..SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB {
            assert_eq!(Ok(unsafe { ptr_a.byte_add(i * 8) }), alloc.allocate(Layout::new::<u64>()));
        }

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have a slab").as_ptr() };
            assert_eq!(alloc.slabs.first_free, None);

            assert_eq!(slab_info.next, None);
            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, 0);
            assert_eq!(slab_info.free.count(), 0);
            assert_eq!(ptr_a.cast(), slab_info.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB)
            );
        }

        let ptr_b = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have 2 slabs").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
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
                (
                    SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB + 1,
                    SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB * 2
                )
            );
        }

        unsafe {
            alloc.deallocate(ptr_a.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have 2 slabs").as_ptr() };

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert!(!slab_info.free.get(0));
            assert_eq!(ptr_b.cast(), slab_info.ptr);

            let slab_info_2 = unsafe { &*slab_info.next.expect("should have 2 slabs").as_ptr() };

            assert_eq!(alloc.slabs.first_free, Some(NonNull::from(slab_info_2)));
            assert_eq!(slab_info_2.next, None);
            assert_eq!(slab_info_2.next_free, Some(NonNull::from(slab_info)));

            assert_eq!(slab_info_2.num_free as usize, 1);
            assert_eq!(slab_info_2.free.count(), 1);
            assert!(slab_info_2.free.get(0));
            assert_eq!(ptr_a.cast(), slab_info_2.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB * 2)
            );
        }

        let ptr_c = alloc.allocate(Layout::new::<u64>()).expect("allocation failure in slab");
        assert_eq!(ptr_c, ptr_a);

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have 2 slabs").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB - 1);
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
                (
                    SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB + 1,
                    SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB * 2
                )
            );
        }

        unsafe {
            alloc.deallocate(ptr_b.cast(), Layout::new::<u64>());
        }

        {
            let alloc = alloc.lock();

            let slab_info = unsafe { &*alloc.slabs.first.expect("should have 2 slabs").as_ptr() };
            assert_eq!(alloc.slabs.first_free, alloc.slabs.first);

            assert_eq!(slab_info.next_free, None);

            assert_eq!(slab_info.num_free as usize, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB);
            assert_eq!(slab_info.free.count(), SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB);
            assert_eq!(ptr_b.cast(), slab_info.ptr);

            let slab_info_2 = unsafe { &*slab_info.next.expect("should have 2 slabs").as_ptr() };

            assert_eq!(slab_info_2.next, None);
            assert_eq!(slab_info_2.next_free, None);

            assert_eq!(slab_info_2.num_free as usize, 0);
            assert_eq!(slab_info_2.free.count(), 0);
            assert_eq!(ptr_a.cast(), slab_info_2.ptr);

            assert_eq!(
                alloc.count(),
                (SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB, SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB * 2)
            );
        }

        unsafe {
            for i in 0..SlabAlloc::<[u8; 8]>::OBJECTS_PER_SLAB {
                alloc.deallocate(ptr_a.byte_add(i * 8).cast(), Layout::new::<u64>());
            }
        }
    }
}
