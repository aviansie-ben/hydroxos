//! The HydroxOS scheduler.
//!
//! This module contains the kernel's scheduler, which is responsible for keeping track of processes and threads running on the machine and
//! facilitating context switching between them from interrupt handlers.

use core::cell::UnsafeCell;
use core::mem;

use crate::arch::interrupt::InterruptFrame;

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
pub(crate) unsafe fn end_interrupt() {
    *IN_INTERRUPT.get() = false;
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

                mem::drop(old_thread_lock);
                let mut old_process_lock = old_process.lock();
                let old_thread_lock = old_thread.lock();

                old_process_lock.enqueue_ready_thread(old_thread_lock);
            },
            task::ThreadState::Dead => {
                // TODO Free thread memory
            },
            _ => {}
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
    use core::sync::atomic::{AtomicBool, Ordering};

    use super::task::*;
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
}
