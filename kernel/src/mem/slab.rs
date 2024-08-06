use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

use super::frame::{self, FrameAllocator};
use super::virt::VirtualAllocRegion;
use super::PageBasedAlloc;
use crate::arch::page::{AddressSpace, PAGE_SIZE};
use crate::arch::{PhysAddr, VirtAddr};
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
            free: {
                let mut free = FixedBitVector::new();

                for i in 0..n {
                    free.set(i as usize, true);
                }

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

pub struct SlabAlloc<const SIZE: usize, const OWN_INFO: bool = false>
where
    [(); pages_per_slab(SIZE)]:,
{
    first: Option<NonNull<SlabInfo>>,
    first_free: Option<NonNull<SlabInfo>>,
}

unsafe impl<const SIZE: usize, const OWN_INFO: bool> Send for SlabAlloc<SIZE, OWN_INFO> where [(); pages_per_slab(SIZE)]: {}

impl<const SIZE: usize, const OWN_INFO: bool> SlabAlloc<SIZE, OWN_INFO>
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
            } else if let Some(slab_ptr) = SLAB_INFO.lock().alloc() {
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

impl<const SIZE: usize, const OWN_INFO: bool> Drop for SlabAlloc<SIZE, OWN_INFO>
where
    [(); pages_per_slab(SIZE)]:,
{
    fn drop(&mut self) {
        let mut next_info = self.first;

        while let Some(mut slab) = next_info {
            let slab = unsafe { slab.as_mut() };

            // NOTE: Must read these *before* we deallocate anything, since slab will be inside
            //       these pages when OWN_INFO is true.
            let ptr = VirtAddr::from_ptr(slab.ptr.as_ptr());
            let ptr_end = ptr + Self::PAGES_PER_SLAB * PAGE_SIZE;
            next_info = slab.next;

            let mut addrspace = AddressSpace::kernel();
            let mut frames = [PhysAddr::zero(); pages_per_slab(SIZE)];

            for (i, f) in frames.iter_mut().enumerate().take(Self::PAGES_PER_SLAB) {
                *f = addrspace.get_page(ptr + i * PAGE_SIZE).expect("slab not mapped").0;
            }

            unsafe {
                frame::get_allocator().free_many(&frames);
                addrspace.virtual_alloc().free(VirtualAllocRegion::new(ptr, ptr_end));
            }

            if !OWN_INFO {
                unsafe {
                    SLAB_INFO.lock().free(NonNull::from(slab).cast());
                }
            }
        }
    }
}

unsafe impl<const SIZE: usize, const OWN_INFO: bool> Allocator for UninterruptibleSpinlock<SlabAlloc<SIZE, OWN_INFO>>
where
    [(); pages_per_slab(SIZE)]:,
{
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() > SIZE || SIZE.next_multiple_of(layout.align()) != SIZE {
            return Err(AllocError);
        }

        match self.lock().alloc() {
            Some(ptr) => Ok(ptr),
            None => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        self.lock().free(ptr.cast())
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

pub static SLAB_INFO: UninterruptibleSpinlock<SlabAlloc<{ core::mem::size_of::<SlabInfo>() }, true>> =
    UninterruptibleSpinlock::new(SlabAlloc::new());

pub static SLAB_8: UninterruptibleSpinlock<SlabAlloc<8>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_16: UninterruptibleSpinlock<SlabAlloc<16>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_32: UninterruptibleSpinlock<SlabAlloc<32>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_64: UninterruptibleSpinlock<SlabAlloc<64>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_128: UninterruptibleSpinlock<SlabAlloc<128>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_256: UninterruptibleSpinlock<SlabAlloc<256>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_512: UninterruptibleSpinlock<SlabAlloc<512>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_1024: UninterruptibleSpinlock<SlabAlloc<1024>> = UninterruptibleSpinlock::new(SlabAlloc::new());
pub static SLAB_2048: UninterruptibleSpinlock<SlabAlloc<2048>> = UninterruptibleSpinlock::new(SlabAlloc::new());
