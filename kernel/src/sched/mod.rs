//! The HydroxOS scheduler.
//!
//! This module contains the kernel's scheduler, which is responsible for keeping track of processes and threads running on the machine and
//! facilitating context switching between them from interrupt handlers.

use alloc::boxed::Box;
use alloc::collections::vec_deque::VecDeque;
use core::cell::UnsafeCell;

use self::task::{Process, Thread};
use crate::{arch::interrupt::{self, InterruptFrame}, sync::uninterruptible::InterruptDisabler};

pub mod task;
pub mod wait;

/// Initializes the scheduler data structures.
///
/// # Safety
///
/// This function should only be called once from the bootstrap process early during the boot process.
pub unsafe fn init() {
    task::Process::init_kernel_process();
}

#[thread_local]
static IN_INTERRUPT: UnsafeCell<bool> = UnsafeCell::new(false);

#[thread_local]
static SOFT_INTERRUPTS: UnsafeCell<VecDeque<Box<dyn FnOnce()>>> = UnsafeCell::new(VecDeque::new());

/// Notifies the scheduler that an asynchronous hardware interrupt handler has begun.
///
/// # Safety
///
/// This method must be called by the architecture's interrupt handling infrastructure before beginning to service an asynchronous hardware
/// interrupt. Failing to call this method or calling it when an asynchronous hardware interrupt is not about to be handled produces
/// undefined behaviour.
#[allow(unused)]
pub(crate) unsafe fn begin_interrupt() {
    *IN_INTERRUPT.get() = true;
}

/// Notifies the scheduler that an asynchronous hardware interrupt handler has ended.
///
/// # Safety
///
/// This method must be called by the architecture's interrupt handling infrastructure after completing the handler for an asynchronous
/// hardware interrupt. Failing to call this method or calling it when an asynchronous hardware interrupt is not about to be completed
/// produces undefined behaviour.
#[allow(unused)]
pub(crate) unsafe fn end_interrupt(interrupt_frame: &mut InterruptFrame) {
    run_soft_interrupts();

    // The interrupt may have caused a Thread to wake up, so if this core is currently idle, attempt a context switch immediately to
    // ensure we aren't sitting around doing nothing for no reason.
    if Thread::current_interrupted().is_none() && Process::is_initialized() {
        perform_context_switch_interrupt(None, interrupt_frame);
    }

    *IN_INTERRUPT.get() = false;
}

/// Enqueues a soft interrupt to be run later (either when interrupts would be re-enabled by dropping an InterruptDisabler or at the end
/// of handling the current interrupt). The soft interrupt is always run with interrupts disabled.
///
/// If the call to this function is not within the context of an interrupt and interrupts are currently enabled, then the provided function
/// is called immediately.
///
/// # Panics
///
/// A panic will occur when running the soft interrupt if it attempts to perform a blocking operation.
pub fn enqueue_soft_interrupt<F: FnOnce() + 'static>(f: F) {
    if !is_handling_interrupt() && interrupt::are_enabled() {
        let _interrupts_disabled = InterruptDisabler::new();
        f();
    } else {
        // SAFETY: No references to SOFT_INTERRUPTS can ever leak and no user-provided code runs while it is in use
        unsafe { &mut *SOFT_INTERRUPTS.get() }.push_back(Box::new(f));
    }
}

/// Runs all pending soft interrupts enqueued by [`enqueue_soft_interrupt`].
pub(crate) fn run_soft_interrupts() {
    let _interrupts_disabled = InterruptDisabler::new();

    // SAFETY: No references to SOFT_INTERRUPTS can ever leak and no user-provided code runs while it is in use
    while let Some(f) = unsafe { &mut *SOFT_INTERRUPTS.get() }.pop_front() {
        f();
    }
}

/// Gets a flag indicating whether an asynchronous hardware interrupt is currently being serviced on this CPU core.
pub fn is_handling_interrupt() -> bool {
    // SAFETY: The value of IN_INTERRUPT can never change during a read. While an interrupt could theoretically occur during the read and
    //         modify its value, the value will always be set back to what it was before when returning to this code.
    unsafe { *IN_INTERRUPT.get() }
}

pub unsafe fn perform_context_switch_interrupt(old_thread_lock: Option<task::ThreadLock>, interrupt_frame: &mut InterruptFrame) {
    assert!(is_handling_interrupt());

    if let Some(mut old_thread_lock) = old_thread_lock {
        debug_assert!(!matches!(*old_thread_lock.state(), task::ThreadState::Running));

        old_thread_lock.save_cpu_state(interrupt_frame);

        match *old_thread_lock.state() {
            task::ThreadState::Ready => {
                let old_thread = old_thread_lock.thread();
                let old_process = old_thread.process().upgrade().unwrap();

                drop(old_thread_lock);
                let mut old_process_lock = old_process.lock();
                let old_thread_lock = old_thread.lock();

                old_process_lock.enqueue_ready_thread(old_thread_lock);
            },
            task::ThreadState::Dead => {
                // TODO Free thread memory
            },
            _ => {},
        }
    }

    // TODO Support user-mode processes
    let thread = task::Process::kernel().lock().dequeue_ready_thread();

    if let Some(ref thread) = thread {
        let mut thread = thread.lock();

        debug_assert!(matches!(*thread.state(), task::ThreadState::Ready));

        *thread.state_mut() = task::ThreadState::Running;
        thread.restore_cpu_state(interrupt_frame);
    } else {
        interrupt_frame.set_to_idle();
    }

    *task::CURRENT_THREAD.get() = thread;
}

#[cfg(test)]
mod test {
    use alloc::rc::Rc;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicBool, Ordering};

    use super::task::*;
    use crate::sync::uninterruptible::InterruptDisabler;
    use crate::test_util::TEST_THREAD_STACK_SIZE;

    #[test_case]
    fn test_thread_basics() {
        let flag = AtomicBool::new(false);
        let thread_fn = || {
            assert!(!flag.load(Ordering::Relaxed));
            flag.store(true, Ordering::Relaxed);
            Thread::yield_current();
            assert!(!flag.load(Ordering::Relaxed));
            flag.store(true, Ordering::Relaxed);
        };

        let thread = unsafe {
            Process::kernel()
                .lock()
                .create_kernel_thread_unchecked(thread_fn, TEST_THREAD_STACK_SIZE)
        };
        thread.lock().wake();

        Thread::yield_current();
        assert!(flag.load(Ordering::Relaxed));
        flag.store(false, Ordering::Relaxed);
        Thread::yield_current();
        assert!(flag.load(Ordering::Relaxed));
        assert!(matches!(*thread.lock().state(), ThreadState::Dead));
    }

    #[test_case]
    fn test_soft_interrupt_in_interrupt_disabler() {
        let flag = Rc::new(Cell::new(false));
        let flag_clone = Rc::clone(&flag);
        let interrupt_disabler = InterruptDisabler::new();

        super::enqueue_soft_interrupt(move || {
            flag_clone.set(true);
        });
        assert!(!flag.get());
        assert_eq!(2, Rc::strong_count(&flag));

        drop(interrupt_disabler);
        assert!(flag.get());
        assert_eq!(1, Rc::strong_count(&flag));
    }
}
