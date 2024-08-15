use alloc::alloc::handle_alloc_error;
use core::alloc::{AllocError, Allocator, GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, Ordering};

use frame::FrameAllocator;
use virt::VirtualAllocRegion;

use crate::arch::page::{AddressSpace, PageFlags, PAGE_SIZE};
use crate::arch::VirtAddr;

pub mod early;
pub mod frame;
pub mod slab;
pub mod virt;

pub struct PageBasedAlloc;

unsafe impl Allocator for PageBasedAlloc {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            return Ok(NonNull::from_raw_parts(NonNull::dangling(), 0));
        }

        if layout.align() > PAGE_SIZE {
            return Err(AllocError);
        }

        let mut addrspace = AddressSpace::kernel();
        let mut frames = [MaybeUninit::uninit(); 16];
        let num_pages = layout.size().div_ceil(PAGE_SIZE);

        let virt_region = if let Some(virt_region) = addrspace.virtual_alloc().alloc(num_pages * PAGE_SIZE) {
            virt_region
        } else {
            return Err(AllocError);
        };
        let start_ptr = virt_region.start();

        let mut num_pages_allocated = 0;
        while num_pages_allocated < num_pages {
            let batch_num_pages = (num_pages - num_pages_allocated).min(16);
            let frames = if let Some(frames) = frame::get_allocator().alloc_many(&mut frames[..batch_num_pages]) {
                frames
            } else {
                let mut num_pages_freed = 0;
                while num_pages_freed < num_pages_allocated {
                    let batch_num_frames = (num_pages_allocated - num_pages_freed).min(16);

                    for (i, f) in frames.iter_mut().enumerate().take(batch_num_frames) {
                        *f = MaybeUninit::new(addrspace.get_page(start_ptr + (num_pages_freed + i) * PAGE_SIZE).unwrap().0);
                    }

                    unsafe {
                        frame::get_allocator().free_many(MaybeUninit::slice_assume_init_ref(&frames[..batch_num_frames]));
                    }

                    num_pages_freed += batch_num_frames;
                }

                unsafe {
                    for i in 0..num_pages_freed {
                        addrspace.set_page_kernel(start_ptr + i * PAGE_SIZE, None);
                    }
                    addrspace.virtual_alloc().free(virt_region);
                }

                return Err(AllocError);
            };

            for (i, &frame) in frames.iter().enumerate() {
                unsafe {
                    let page_ptr = start_ptr + (num_pages_allocated + i) * PAGE_SIZE;

                    assert_eq!(addrspace.get_page(page_ptr), None);
                    addrspace.set_page_kernel(page_ptr, Some((frame, PageFlags::WRITEABLE)));
                }
            }

            num_pages_allocated += batch_num_pages;
        }

        Ok(NonNull::from_raw_parts(
            NonNull::new(start_ptr.as_mut_ptr()).unwrap(),
            num_pages * PAGE_SIZE,
        ))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let ptr = VirtAddr::from_ptr(ptr.as_ptr());
        let mut addrspace = AddressSpace::kernel();
        let mut frames = [MaybeUninit::uninit(); 16];
        let num_pages = layout.size().div_ceil(PAGE_SIZE);

        let mut num_pages_freed = 0;
        while num_pages_freed < num_pages {
            let batch_num_frames = (num_pages - num_pages_freed).min(16);

            for (i, f) in frames.iter_mut().enumerate().take(batch_num_frames) {
                let page = ptr + (num_pages_freed + i) * PAGE_SIZE;
                *f = MaybeUninit::new(addrspace.get_page(page).unwrap().0);
                addrspace.set_page_kernel(page, None);
            }

            unsafe {
                frame::get_allocator().free_many(MaybeUninit::slice_assume_init_ref(&frames[..batch_num_frames]));
            }

            num_pages_freed += batch_num_frames;
        }

        unsafe {
            addrspace
                .virtual_alloc()
                .free(VirtualAllocRegion::new(ptr, ptr + num_pages * PAGE_SIZE));
        }
    }

    unsafe fn grow(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if new_layout.align() >= PAGE_SIZE {
            return Err(AllocError);
        }

        let num_pages_old = old_layout.size().div_ceil(PAGE_SIZE);
        let num_pages_new = new_layout.size().div_ceil(PAGE_SIZE);

        if num_pages_new == num_pages_old {
            return Ok(NonNull::from_raw_parts(ptr.cast(), num_pages_new * PAGE_SIZE));
        }

        let new_ptr = self.allocate(new_layout)?;

        unsafe {
            ptr::copy_nonoverlapping::<u8>(ptr.as_ptr(), new_ptr.as_mut_ptr(), old_layout.size());
            self.deallocate(ptr, old_layout);
        }

        Ok(new_ptr)
    }

    unsafe fn shrink(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if new_layout.align() >= PAGE_SIZE {
            return Err(AllocError);
        }

        let num_pages_old = old_layout.size().div_ceil(PAGE_SIZE);
        let num_pages_new = new_layout.size().div_ceil(PAGE_SIZE);

        if num_pages_new != num_pages_old {
            let end_ptr = VirtAddr::from_ptr(ptr.as_ptr()) + num_pages_new * PAGE_SIZE;
            let mut addrspace = AddressSpace::kernel();
            let mut frames = [MaybeUninit::uninit(); 16];
            let num_pages = num_pages_old - num_pages_new;

            let mut num_pages_freed = 0;
            while num_pages_freed < num_pages {
                let batch_num_frames = (num_pages - num_pages_freed).min(16);

                for (i, f) in frames.iter_mut().enumerate().take(batch_num_frames) {
                    *f = MaybeUninit::new(addrspace.get_page(end_ptr + (num_pages_freed + i) * PAGE_SIZE).unwrap().0);
                }

                unsafe {
                    frame::get_allocator().free_many(MaybeUninit::slice_assume_init_ref(&frames[..batch_num_frames]));
                }

                num_pages_freed += batch_num_frames;
            }

            unsafe {
                addrspace
                    .virtual_alloc()
                    .free(VirtualAllocRegion::new(end_ptr, end_ptr + num_pages * PAGE_SIZE));
            }
        }

        Ok(NonNull::from_raw_parts(ptr.cast(), num_pages_new * PAGE_SIZE))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AllocType {
    Early,
    Slab8,
    Slab16,
    Slab32,
    Slab64,
    Slab128,
    Slab256,
    Slab512,
    Slab1024,
    Slab2048,
    Page,
}

static USE_EARLY_ALLOC: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_use_early_alloc(use_early_alloc: bool) {
    USE_EARLY_ALLOC.store(use_early_alloc, Ordering::Release);
}

fn get_new_alloc_type(layout: Layout) -> AllocType {
    if USE_EARLY_ALLOC.load(Ordering::Acquire) {
        AllocType::Early
    } else {
        match layout.size().max(layout.align()) {
            0..=8 => AllocType::Slab8,
            9..=16 => AllocType::Slab16,
            17..=32 => AllocType::Slab32,
            33..=64 => AllocType::Slab64,
            65..=128 => AllocType::Slab128,
            129..=256 => AllocType::Slab256,
            257..=512 => AllocType::Slab512,
            513..=1024 => AllocType::Slab1024,
            1025..=2048 => AllocType::Slab2048,
            2049.. => AllocType::Page,
        }
    }
}

fn get_existing_alloc_type(ptr: *mut u8, layout: Layout) -> AllocType {
    if early::is_in_early_alloc_region(ptr) {
        AllocType::Early
    } else {
        get_new_alloc_type(layout)
    }
}

pub struct DefaultAlloc;

unsafe impl GlobalAlloc for DefaultAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = match get_new_alloc_type(layout) {
            AllocType::Early => Ok(NonNull::from_raw_parts(
                NonNull::new(early::alloc(layout.size(), layout.align())).unwrap().cast(),
                layout.size(),
            )),
            AllocType::Slab8 => slab::SLAB_8.allocate(layout),
            AllocType::Slab16 => slab::SLAB_16.allocate(layout),
            AllocType::Slab32 => slab::SLAB_32.allocate(layout),
            AllocType::Slab64 => slab::SLAB_64.allocate(layout),
            AllocType::Slab128 => slab::SLAB_128.allocate(layout),
            AllocType::Slab256 => slab::SLAB_256.allocate(layout),
            AllocType::Slab512 => slab::SLAB_512.allocate(layout),
            AllocType::Slab1024 => slab::SLAB_1024.allocate(layout),
            AllocType::Slab2048 => slab::SLAB_2048.allocate(layout),
            AllocType::Page => PageBasedAlloc.allocate(layout),
        };

        match result {
            Ok(ptr) => ptr.as_mut_ptr(),
            Err(_) => {
                handle_alloc_error(layout);
            },
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let ptr = NonNull::new(ptr).unwrap();

        match get_existing_alloc_type(ptr.as_ptr(), layout) {
            AllocType::Early => early::free(ptr.as_ptr(), layout.size()),
            AllocType::Slab8 => slab::SLAB_8.deallocate(ptr, layout),
            AllocType::Slab16 => slab::SLAB_16.deallocate(ptr, layout),
            AllocType::Slab32 => slab::SLAB_32.deallocate(ptr, layout),
            AllocType::Slab64 => slab::SLAB_64.deallocate(ptr, layout),
            AllocType::Slab128 => slab::SLAB_128.deallocate(ptr, layout),
            AllocType::Slab256 => slab::SLAB_256.deallocate(ptr, layout),
            AllocType::Slab512 => slab::SLAB_512.deallocate(ptr, layout),
            AllocType::Slab1024 => slab::SLAB_1024.deallocate(ptr, layout),
            AllocType::Slab2048 => slab::SLAB_2048.deallocate(ptr, layout),
            AllocType::Page => PageBasedAlloc.deallocate(ptr, layout),
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe fn realloc<A: Allocator>(alloc: &A, ptr: NonNull<u8>, layout: Layout, new_size: usize) -> Result<NonNull<[u8]>, AllocError> {
            if new_size >= layout.size() {
                alloc.grow(ptr, layout, Layout::from_size_align_unchecked(new_size, layout.align()))
            } else {
                alloc.shrink(ptr, layout, Layout::from_size_align_unchecked(new_size, layout.align()))
            }
        }

        let old_ty = get_existing_alloc_type(ptr, layout);
        let new_ty = get_new_alloc_type(Layout::from_size_align(new_size, layout.align()).unwrap());

        if new_ty == old_ty {
            let ptr = NonNull::new(ptr).unwrap();
            let result = match get_existing_alloc_type(ptr.as_ptr(), layout) {
                AllocType::Early => Ok(NonNull::from_raw_parts(
                    NonNull::new(early::realloc(ptr.as_ptr(), layout.size(), new_size)).unwrap().cast(),
                    layout.size(),
                )),
                AllocType::Slab8 => realloc(&slab::SLAB_8, ptr, layout, new_size),
                AllocType::Slab16 => realloc(&slab::SLAB_16, ptr, layout, new_size),
                AllocType::Slab32 => realloc(&slab::SLAB_32, ptr, layout, new_size),
                AllocType::Slab64 => realloc(&slab::SLAB_64, ptr, layout, new_size),
                AllocType::Slab128 => realloc(&slab::SLAB_128, ptr, layout, new_size),
                AllocType::Slab256 => realloc(&slab::SLAB_256, ptr, layout, new_size),
                AllocType::Slab512 => realloc(&slab::SLAB_512, ptr, layout, new_size),
                AllocType::Slab1024 => realloc(&slab::SLAB_1024, ptr, layout, new_size),
                AllocType::Slab2048 => realloc(&slab::SLAB_2048, ptr, layout, new_size),
                AllocType::Page => realloc(&PageBasedAlloc, ptr, layout, new_size),
            };

            match result {
                Ok(ptr) => ptr.as_mut_ptr(),
                Err(_) => {
                    handle_alloc_error(layout);
                },
            }
        } else {
            let new_ptr = self.alloc(Layout::from_size_align_unchecked(new_size, layout.align()));

            ptr::copy_nonoverlapping(ptr, new_ptr, layout.size().min(new_size));
            self.dealloc(ptr, layout);

            new_ptr
        }
    }
}

#[global_allocator]
pub static ALLOCATOR: DefaultAlloc = DefaultAlloc;
