use crate::x86_64::page::get_phys_mem_base;

const NUM_FRAMES_PER_PAGE: usize = crate::x86_64::page::PAGE_SIZE / core::mem::size_of::<x86_64::PhysAddr>();

#[repr(C)]
struct StackFrameAllocatorPage {
    frames: [x86_64::PhysAddr; NUM_FRAMES_PER_PAGE]
}

pub struct StackFrameAllocator {
    num_frames_available: usize,
    stack_top: *mut StackFrameAllocatorPage
}

impl StackFrameAllocator {
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

    pub unsafe fn push(&mut self, frame: x86_64::PhysAddr) {
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

    pub fn pop(&mut self) -> Option<x86_64::PhysAddr> {
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

    pub fn num_frames_available(&self) -> usize {
        self.num_frames_available
    }
}

unsafe impl Send for StackFrameAllocator {}

#[cfg(test)]
mod tests {
    use x86_64::structures::paging::mapper::{OffsetPageTable, Translate, TranslateResult};
    use x86_64::{PhysAddr, VirtAddr};

    use super::{StackFrameAllocator, NUM_FRAMES_PER_PAGE};
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
        assert_eq!(None, allocator.pop());
    }

    #[test_case]
    fn test_push_pop_one() {
        unsafe {
            let mut allocator = StackFrameAllocator::new();

            allocator.push(get_test_page(0));

            assert_eq!(1, allocator.num_frames_available());
            assert_eq!(Some(get_test_page(0)), allocator.pop());

            assert_eq!(0, allocator.num_frames_available());
            assert_eq!(None, allocator.pop());
        };
    }

    #[test_case]
    fn test_push_pop_many() {
        unsafe {
            let mut allocator = StackFrameAllocator::new();

            for i in 0..(TEST_AREA.len() - 1) {
                for _ in (0..NUM_FRAMES_PER_PAGE).step_by(2) {
                    allocator.push(get_test_page(i));
                    allocator.push(get_test_page(i + 1));
                }

                assert_eq!((i + 1) * NUM_FRAMES_PER_PAGE, allocator.num_frames_available());
            }

            for i in (0..(TEST_AREA.len() - 1)).rev() {
                for _ in (0..NUM_FRAMES_PER_PAGE).step_by(2) {
                    assert_eq!(Some(get_test_page(i + 1)), allocator.pop());
                    assert_eq!(Some(get_test_page(i)), allocator.pop());
                }

                assert_eq!(i * NUM_FRAMES_PER_PAGE, allocator.num_frames_available());
            }

            assert_eq!(None, allocator.pop());
        };
    }
}
