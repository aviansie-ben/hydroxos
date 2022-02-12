//! Physical frame allocation.

use core::mem::MaybeUninit;

use bootloader::BootInfo;
use bootloader::bootinfo::MemoryRegionType;

use crate::sync::uninterruptible::{UninterruptibleSpinlock, UninterruptibleSpinlockGuard};
use crate::x86_64::page::get_phys_mem_base;

const NUM_FRAMES_PER_PAGE: usize = crate::x86_64::page::PAGE_SIZE / core::mem::size_of::<x86_64::PhysAddr>();

/// An allocator that returns physical page frames.
pub trait FrameAllocator {
    /// Free a single page frame and make it available through this allocator.
    ///
    /// # Safety
    ///
    /// This method assumes that the provided page frame is valid (i.e. it points at memory that's usable as general-purpose RAM) and that
    /// it is no longer in use by anything else. Providing valid memory is required, even if callers to [`FrameAllocator::alloc_one`] don't
    /// require this, as the frame allocator is free to use this memory internally as scratch memory.
    unsafe fn free_one(&mut self, frame: x86_64::PhysAddr);

    /// Allocates a single page frame from this allocator. Returns [`None`] if no page frames are available.
    ///
    /// # Safety
    ///
    /// Unless the particular allocator in use makes claims otherwise, page frames received from this API may contain sensitive information
    /// that was left in memory when the page frame was freed. Before making any memory allocated from here visible to user-space code, it
    /// should be initialized to a known pattern to avoid leaking information to untrusted code.
    fn alloc_one(&mut self) -> Option<x86_64::PhysAddr>;

    /// Gets the total number of page frames available from this allocator.
    fn num_frames_available(&self) -> usize;

    /// Frees multiple page frames and makes them available through this allocator.
    ///
    /// # Safety
    ///
    /// This method assumes that all provided page frames are valid (i.e. they point at memory that's usable as general-purpose RAM), that
    /// they are no longer in use by anything else, and that the list of page frames to free does not contain duplicate entries.
    unsafe fn free_many(&mut self, frames: &[x86_64::PhysAddr]) {
        for &frame in frames.iter() {
            self.free_one(frame);
        }
    }

    /// Allocates multiple page frames from this allocator. Returns [`None`] (and does not allocate any page frames) if insufficient page
    /// frames are available.
    ///
    /// Upon success, `frames_out` will be initialized with the addresses of the page frames that were allocated and a slice viewing
    /// `frames_out` as initialized will be returned.
    ///
    /// # Safety
    ///
    /// This method has the same guarantees with regards to memory initialization as [`FrameAllocator::alloc_one`].
    fn alloc_many<'a>(&mut self, frames_out: &'a mut [MaybeUninit<x86_64::PhysAddr>]) -> Option<&'a mut [x86_64::PhysAddr]> {
        if self.num_frames_available() < frames_out.len() {
            return None;
        }

        for frame_out in frames_out.iter_mut() {
            unsafe { *frame_out.as_mut_ptr() = self.alloc_one().unwrap() }
        }

        Some(unsafe { &mut *(frames_out as *mut [MaybeUninit<x86_64::PhysAddr>] as *mut [x86_64::PhysAddr]) })
    }
}

#[repr(C)]
struct StackFrameAllocatorPage {
    frames: [x86_64::PhysAddr; NUM_FRAMES_PER_PAGE]
}

/// A page frame allocator that maintains an internal stack of free frames.
pub struct StackFrameAllocator {
    num_frames_available: usize,
    stack_top: *mut StackFrameAllocatorPage
}

impl StackFrameAllocator {
    /// Creates a new empty stack page frame allocator.
    pub const fn new() -> StackFrameAllocator {
        StackFrameAllocator {
            num_frames_available: 0,
            stack_top: core::ptr::null_mut()
        }
    }

    fn frames_on_top_stack_frame(&self) -> usize {
        assert_ne!(self.num_frames_available, 0);

        let n = self.num_frames_available % NUM_FRAMES_PER_PAGE;

        if n == 0 {
            NUM_FRAMES_PER_PAGE
        } else {
            n
        }
    }
}

impl FrameAllocator for StackFrameAllocator {
    unsafe fn free_one(&mut self, frame: x86_64::PhysAddr) {
        if self.num_frames_available == 0 {
            self.stack_top = get_phys_mem_base().offset(frame.as_u64() as isize) as *mut StackFrameAllocatorPage;
            (*self.stack_top).frames[0] = x86_64::PhysAddr::zero();
        } else {
            let i = self.frames_on_top_stack_frame();

            if i == NUM_FRAMES_PER_PAGE {
                let new_stack_top = get_phys_mem_base().offset(frame.as_u64() as isize) as *mut StackFrameAllocatorPage;

                (*new_stack_top).frames[0] = x86_64::PhysAddr::new((self.stack_top as *mut u8).offset_from(get_phys_mem_base()) as u64);
                self.stack_top = new_stack_top;
            } else {
                (*self.stack_top).frames[i] = frame;
            };
        };

        self.num_frames_available += 1;
    }

    fn alloc_one(&mut self) -> Option<x86_64::PhysAddr> {
        unsafe {
            if self.num_frames_available == 0 {
                None
            } else {
                let i = self.frames_on_top_stack_frame();
                let result = if i == 1 {
                    let old_stack_top = self.stack_top;

                    self.stack_top = if self.num_frames_available == 1 {
                        core::ptr::null_mut()
                    } else {
                        get_phys_mem_base().offset((*self.stack_top).frames[0].as_u64() as isize) as *mut StackFrameAllocatorPage
                    };
                    x86_64::PhysAddr::new((old_stack_top as *mut u8).offset_from(get_phys_mem_base()) as u64)
                } else {
                    (*self.stack_top).frames[i - 1]
                };

                self.num_frames_available -= 1;
                Some(result)
            }
        }
    }

    fn num_frames_available(&self) -> usize {
        self.num_frames_available
    }
}

unsafe impl Send for StackFrameAllocator {}

pub struct LockFrameAllocator<T: FrameAllocator>(UninterruptibleSpinlock<T>);

impl <T: FrameAllocator> LockFrameAllocator<T> {
    pub const fn new(alloc: T) -> LockFrameAllocator<T> {
        LockFrameAllocator(UninterruptibleSpinlock::new(alloc))
    }

    pub fn lock(&self) -> UninterruptibleSpinlockGuard<T> {
        self.0.lock()
    }
}

impl <T: FrameAllocator> FrameAllocator for &'_ LockFrameAllocator<T> {
    unsafe fn free_one(&mut self, frame: x86_64::PhysAddr) {
        self.lock().free_one(frame);
    }

    fn alloc_one(&mut self) -> Option<x86_64::PhysAddr> {
        self.lock().alloc_one()
    }

    fn num_frames_available(&self) -> usize {
        self.lock().num_frames_available()
    }

    unsafe fn free_many(&mut self, frames: &[x86_64::PhysAddr]) {
        self.lock().free_many(frames)
    }

    fn alloc_many<'a>(&mut self, frames_out: &'a mut [MaybeUninit<x86_64::PhysAddr>]) -> Option<&'a mut [x86_64::PhysAddr]> {
        self.lock().alloc_many(frames_out)
    }
}

static FRAME_ALLOC: LockFrameAllocator<StackFrameAllocator> = LockFrameAllocator::new(StackFrameAllocator::new());

pub fn get_allocator() -> &'static LockFrameAllocator<impl FrameAllocator> {
    &FRAME_ALLOC
}

fn is_free(region_ty: MemoryRegionType) -> bool {
    match region_ty {
        MemoryRegionType::Usable => true,
        MemoryRegionType::Bootloader => true,
        _ => false
    }
}

fn is_usable(region_ty: MemoryRegionType) -> bool {
    match region_ty {
        MemoryRegionType::Usable => true,
        MemoryRegionType::InUse => true,
        MemoryRegionType::AcpiReclaimable => true,
        MemoryRegionType::Kernel => true,
        MemoryRegionType::KernelStack => true,
        MemoryRegionType::PageTable => true,
        MemoryRegionType::Bootloader => true,
        MemoryRegionType::BootInfo => true,
        MemoryRegionType::Package => true,
        _ => false
    }
}

pub unsafe fn init(boot_info: &BootInfo) -> usize {
    let mut num_frames = 0;
    let mut frame_alloc = get_allocator().lock();

    for region in boot_info.memory_map.iter() {
        if is_free(region.region_type) {
            for frame_n in region.range.start_frame_number..region.range.end_frame_number {
                frame_alloc.free_one(x86_64::PhysAddr::new(frame_n * crate::x86_64::page::PAGE_SIZE as u64));
            };
        };

        if is_usable(region.region_type) {
            num_frames += region.range.end_frame_number - region.range.start_frame_number;
        };
    };

    num_frames.try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use core::mem::MaybeUninit;

    use x86_64::structures::paging::mapper::{OffsetPageTable, Translate, TranslateResult};
    use x86_64::{PhysAddr, VirtAddr};

    use super::{FrameAllocator, StackFrameAllocator, NUM_FRAMES_PER_PAGE};
    use crate::util::PageAligned;
    use crate::x86_64::page::{get_phys_mem_base, get_phys_mem_ptr_mut};

    static TEST_AREA: PageAligned<[[u8; crate::x86_64::page::PAGE_SIZE]; 10]> = PageAligned::new([[0; crate::x86_64::page::PAGE_SIZE]; 10]);

    unsafe fn get_test_page(idx: usize) -> PhysAddr {
        let addr = get_phys_mem_ptr_mut(x86_64::registers::control::Cr3::read().0.start_address());
        let table = OffsetPageTable::new(&mut *addr, VirtAddr::new(get_phys_mem_base() as u64));

        match table.translate(VirtAddr::new(TEST_AREA[idx].as_ptr() as u64)) {
            TranslateResult::Mapped { frame, offset, flags: _ } => frame.start_address() + offset,
            TranslateResult::NotMapped => unreachable!(),
            TranslateResult::InvalidFrameAddress(_) => unreachable!()
        }
    }

    #[test_case]
    fn test_pop_empty() {
        let mut allocator = StackFrameAllocator::new();

        assert_eq!(0, allocator.num_frames_available());
        assert_eq!(None, allocator.alloc_one());
    }

    #[test_case]
    fn test_push_pop_one() {
        unsafe {
            let mut allocator = StackFrameAllocator::new();

            allocator.free_one(get_test_page(0));

            assert_eq!(1, allocator.num_frames_available());
            assert_eq!(Some(get_test_page(0)), allocator.alloc_one());

            assert_eq!(0, allocator.num_frames_available());
            assert_eq!(None, allocator.alloc_one());
        };
    }

    #[test_case]
    fn test_push_pop_many() {
        unsafe {
            let mut allocator = StackFrameAllocator::new();

            for i in 0..(TEST_AREA.len() - 1) {
                for _ in (0..NUM_FRAMES_PER_PAGE).step_by(2) {
                    allocator.free_many(&[get_test_page(i), get_test_page(i + 1)]);
                }

                assert_eq!((i + 1) * NUM_FRAMES_PER_PAGE, allocator.num_frames_available());
            }

            for i in (0..(TEST_AREA.len() - 1)).rev() {
                for _ in (0..NUM_FRAMES_PER_PAGE).step_by(2) {
                    let mut frames = [MaybeUninit::uninit(); 2];

                    assert_eq!(
                        Some(&mut [get_test_page(i + 1), get_test_page(i)][..]),
                        allocator.alloc_many(&mut frames)
                    );
                }

                assert_eq!(i * NUM_FRAMES_PER_PAGE, allocator.num_frames_available());
            }

            assert_eq!(None, allocator.alloc_one());
        };
    }
}
