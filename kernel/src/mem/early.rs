use core::sync::atomic::{AtomicPtr, Ordering};
use core::{cmp, ptr};

use crate::util::{PageAligned, SharedUnsafeCell};

const EARLY_ALLOC_SIZE: usize = 1024 * 1024;

static EARLY_ALLOC_AREA: PageAligned<SharedUnsafeCell<[u8; EARLY_ALLOC_SIZE]>> =
    PageAligned::new(SharedUnsafeCell::new([0; EARLY_ALLOC_SIZE]));
static EARLY_ALLOC_MARK: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());

pub fn init() {
    if EARLY_ALLOC_MARK
        .compare_exchange(
            ptr::null_mut(),
            EARLY_ALLOC_AREA.get() as *mut u8,
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .is_err()
    {
        panic!("Attempt to initialize early memory allocation more than once");
    };
}

fn get_full_size(size: usize) -> u32 {
    if size == 0 {
        4
    } else {
        u32::try_from(size)
            .ok()
            .and_then(|sz| ((sz - 1) & !3).checked_add(8))
            .expect("Early allocation too large")
    }
}

pub fn alloc(size: usize, align: usize) -> *mut u8 {
    // We always need at least 4 byte alignment, since we store the 4 byte allocation size after each block
    let align = u32::try_from(align.max(4)).expect("Early allocation too large");
    let size = get_full_size(size);

    unsafe {
        let early_alloc_end = (*EARLY_ALLOC_AREA.get()).as_mut_ptr_range().end;

        loop {
            let mark = EARLY_ALLOC_MARK.load(Ordering::Relaxed);
            let align_offset = mark.align_offset(align as usize) as u32;
            let alloc_size = size
                .checked_add(align_offset)
                .and_then(|sz| isize::try_from(sz).ok())
                .expect("Early allocation too large");

            if mark.is_null() {
                panic!("Attempt to use early memory allocation before initializing it");
            } else if early_alloc_end.offset_from(mark) < alloc_size {
                panic!("Out of early allocation memory");
            };

            if EARLY_ALLOC_MARK
                .compare_exchange(mark, mark.offset(alloc_size), Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                ptr::write_bytes(mark, 0xAD, (align_offset + size) as usize);
                *(mark.add((align_offset + size - 4) as usize) as *mut u32) = alloc_size as u32;
                break mark.add(align_offset as usize);
            };
        }
    }
}

pub unsafe fn free(ptr: *mut u8, size: usize) {
    ptr::write_bytes(ptr, 0xEA, size as usize);

    let size = get_full_size(size);
    let mark = EARLY_ALLOC_MARK.load(Ordering::Relaxed);

    if mark == ptr.add(size as usize) {
        let real_size = *(ptr.add((size - 4) as usize) as *mut u32) as usize;

        let _ = EARLY_ALLOC_MARK.compare_exchange(mark, mark.sub(real_size), Ordering::Relaxed, Ordering::Relaxed);
    }
}

pub fn is_in_early_alloc_region(ptr: *mut u8) -> bool {
    let ea_ptr = EARLY_ALLOC_AREA.get() as *mut u8;
    let ea_end_ptr = unsafe { ea_ptr.add(EARLY_ALLOC_SIZE) };

    ptr >= ea_ptr && ptr < ea_end_ptr
}

unsafe fn realloc_grow(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    let old_size = get_full_size(old_size);
    let new_size = get_full_size(new_size);
    let mark = EARLY_ALLOC_MARK.load(Ordering::Relaxed);

    if mark == ptr.add(old_size as usize) {
        if (*EARLY_ALLOC_AREA.get()).as_mut_ptr_range().end.offset_from(mark)
            < isize::try_from(new_size - old_size).expect("Early allocation too large")
        {
            panic!("Out of early allocation memory");
        }

        let real_old_size = *(ptr.add((old_size - 4) as usize) as *mut u32);
        let real_new_size = (real_old_size - old_size)
            .checked_add(new_size)
            .expect("Early allocation too large");

        if EARLY_ALLOC_MARK
            .compare_exchange(mark, ptr.add(new_size as usize), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            ptr::write_bytes(ptr.add(old_size as usize), 0xAD, (new_size - old_size) as usize);
            *(ptr.add((new_size - 4) as usize) as *mut u32) = real_new_size;
            return ptr;
        }
    }

    ptr::null_mut()
}

unsafe fn realloc_shrink(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    ptr::write_bytes(ptr.add(new_size), 0xEA, old_size - new_size);

    let old_size = get_full_size(old_size);
    let new_size = get_full_size(new_size);
    let mark = EARLY_ALLOC_MARK.load(Ordering::Relaxed);

    if mark == ptr.add(old_size as usize) {
        let real_old_size = *(ptr.add((old_size - 4) as usize) as *mut u32);
        let real_new_size = real_old_size - old_size + new_size;

        if EARLY_ALLOC_MARK
            .compare_exchange(mark, ptr.add(new_size as usize), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            *(ptr.add((new_size - 4) as usize) as *mut u32) = real_new_size;
        }
    }

    ptr
}

pub unsafe fn realloc(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    match new_size.cmp(&old_size) {
        cmp::Ordering::Greater => realloc_grow(ptr, old_size, new_size),
        cmp::Ordering::Less => realloc_shrink(ptr, old_size, new_size),
        cmp::Ordering::Equal => ptr,
    }
}

pub fn usage() -> (usize, usize) {
    (
        unsafe {
            EARLY_ALLOC_MARK
                .load(Ordering::Relaxed)
                .byte_offset_from(EARLY_ALLOC_AREA.get() as *const u8) as usize
        },
        EARLY_ALLOC_SIZE,
    )
}
