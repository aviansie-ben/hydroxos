use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};

use crate::util::OneShotManualInit;

pub mod early;
pub mod frame;
pub mod virt;

static DONE_EARLY_ALLOC: OneShotManualInit<()> = OneShotManualInit::uninit();

pub struct DefaultAlloc;

unsafe impl GlobalAlloc for DefaultAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !DONE_EARLY_ALLOC.is_init() {
            early::alloc(layout.size(), layout.align())
        } else {
            todo!()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if early::is_in_early_alloc_region(ptr) {
            early::free(ptr, layout.size())
        } else {
            todo!()
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let quick_result = if early::is_in_early_alloc_region(ptr) {
            if !DONE_EARLY_ALLOC.is_init() {
                NonNull::new(early::realloc(ptr, layout.size(), new_size))
            } else {
                None
            }
        } else {
            None // TODO
        };

        if let Some(quick_result) = quick_result {
            quick_result.as_ptr()
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
