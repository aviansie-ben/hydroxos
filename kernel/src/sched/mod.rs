//! The HydroxOS scheduler.
//!
//! This module contains the kernel's scheduler, which is responsible for keeping track of processes and threads running on the machine and
//! facilitating context switching between them from interrupt handlers.

use core::cell::UnsafeCell;

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
pub(crate) unsafe fn end_interrupt() {
    *IN_INTERRUPT.get() = false;
}

/// Gets a flag indicating whether an asynchronous hardware interrupt is currently being serviced on this CPU core.
pub fn is_handling_interrupt() -> bool {
    // SAFETY: The value of IN_INTERRUPT can never change during a read. While an interrupt could theoretically occur during the read and
    //         modify its value, the value will always be set back to what it was before when returning to this code.
    unsafe { *IN_INTERRUPT.get() }
}
