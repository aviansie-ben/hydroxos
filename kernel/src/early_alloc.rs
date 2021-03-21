use core::alloc::{GlobalAlloc, Layout};
use core::convert::TryFrom;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::util::{SharedUnsafeCell, PageAligned};

const EARLY_ALLOC_SIZE: usize = 1 * 1024 * 1024;

static EARLY_ALLOC_AREA: PageAligned<SharedUnsafeCell<[u8; EARLY_ALLOC_SIZE]>> = PageAligned::new(SharedUnsafeCell::new([0; EARLY_ALLOC_SIZE]));
static EARLY_ALLOC_MARK: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

pub fn init() {
    if EARLY_ALLOC_MARK.compare_exchange(core::ptr::null_mut(), EARLY_ALLOC_AREA.get() as *mut u8, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        panic!("Attempt to initialize early memory allocation more than once");
    };
}

pub fn alloc(size: usize, align: usize) -> *mut u8 {
    unsafe {
        if size.checked_add(align).and_then(|sz| isize::try_from(sz).ok()).is_none() {
            panic!("Max allocation size too large");
        };

        let alloc_area = &*EARLY_ALLOC_AREA.get();

        loop {
            let mark = EARLY_ALLOC_MARK.load(Ordering::Relaxed);
            let align_offset = mark.align_offset(align);
            let alloc_size = (size + align_offset) as isize;

            if mark.is_null() {
                panic!("Attempt to use early memory allocation before initializing it");
            } else if alloc_area.as_ptr_range().end.offset_from(mark) < alloc_size {
                panic!("Out of early allocation memory");
            };

            if EARLY_ALLOC_MARK.compare_exchange(mark, mark.offset(alloc_size), Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                break mark.offset(align_offset as isize);
            };
        }
    }
}

pub fn free(_: *mut u8) {
    // TODO Implement once we move away from bump allocation
}

struct EarlyAlloc;

unsafe impl GlobalAlloc for EarlyAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
        free(ptr);
    }
}

#[global_allocator]
static ALLOCATOR: EarlyAlloc = EarlyAlloc;
