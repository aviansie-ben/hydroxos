//! Synchronization primitives suitable for use on data structures accessed from within interrupt handlers.
//!
//! Many data structures in the kernel contain locks that may need to be acquired in an interrupt handler, e.g. buffers used for I/O,
//! scheduler data structures, etc. Using normal spinlocks to implement this could result in a deadlock if the following series of events
//! occurs:
//!
//! - A thread running kernel code acquires the spinlock
//! - An ansynchronous hardware interrupt occurs on the same core as that thread, temporarily interrupting it
//! - The interrupt handler attempts to acquire the same spinlock
//!
//! In this case, the system will deadlock since the thread is waiting for the interrupt to complete before it can continue, while the
//! interrupt handler is waiting for the thread to release the lock before it can complete. This can be corrected by using an
//! uninterruptible spinlock such as [`UninterruptibleSpinlock`], which disable all interrupts on the current CPU core prior to acquiring
//! a lock and re-enable them only upon releasing the last guard on the current CPU core.
//
//! These uninterruptible spinlocks also have the added benefit of avoiding starvation that may occur if an interrupt takes too long to
//! process while a lock is held. Instead, handling of the interrupt will be delayed until after the lock is released and thus avoid
//! potentially forcing other CPU cores to wait while the interrupt is handled.
//!
//! That being said, these spinlocks do have a number of downsides that make them unsuitable for many applications. Locks that guard
//! particularly long critical sections could cause resource starvation, as interrupts will not be handled and context switches to other
//! threads will not occur while the lock is held. These spinlocks are also held by a CPU core, _not_ by a thread, meaning that it is not
//! possible to block the current thread while holding an interrupt-disabling spinlock. Due to these limitations, these spinlocks should
//! only be used for short-lived critical sections where the thread holding the lock will never need to block while keeping the data
//! structure locked.

use core::cell::Cell;
use core::mem;
use core::ops::{Deref, DerefMut};

use x86_64::instructions::interrupts;

#[thread_local]
static INTERRUPT_DISABLER_STATE: Cell<(usize, bool)> = Cell::new((0, false));

/// A guard that keeps interrupts disabled on the current CPU core while it exists.
pub struct InterruptDisabler(());

impl InterruptDisabler {
    /// Create a new interrupt-disabling guards. All interrupts will be disabled on the local CPU core as long as a guard returned from this
    /// function exists.
    pub fn new() -> InterruptDisabler {
        let (n, was_enabled) = INTERRUPT_DISABLER_STATE.get();

        let was_enabled = if n == 0 {
            let was_enabled = interrupts::are_enabled();
            interrupts::disable();
            was_enabled
        } else {
            was_enabled
        };

        INTERRUPT_DISABLER_STATE.set((n + 1, was_enabled));
        InterruptDisabler(())
    }

    /// Get the number of interrupt-disabling guards that currently exist on the local CPU core.
    pub fn num_held() -> usize {
        INTERRUPT_DISABLER_STATE.get().0
    }

    /// Drops this interrupt-disabling guard without actually enabling interrupts. Returns `true` if interrupts would have been enabled had
    /// this guard been dropped normally and `false` otherwise.
    pub fn drop_without_enable(self) -> bool {
        assert!(!interrupts::are_enabled());

        mem::forget(self);

        let (n, was_enabled) = INTERRUPT_DISABLER_STATE.get();
        INTERRUPT_DISABLER_STATE.set((n - 1, was_enabled));

        n == 1 && was_enabled
    }
}

impl !Send for InterruptDisabler {}

impl Drop for InterruptDisabler {
    fn drop(&mut self) {
        assert!(!interrupts::are_enabled());

        let (n, was_enabled) = INTERRUPT_DISABLER_STATE.get();
        INTERRUPT_DISABLER_STATE.set((n - 1, was_enabled));

        if n == 1 && was_enabled {
            interrupts::enable();
        };
    }
}

/// A spinlock that keeps interrupts disabled on the local CPU core while it is locked.
#[derive(Debug)]
pub struct UninterruptibleSpinlock<T>(spin::Mutex<T>);

impl<T> UninterruptibleSpinlock<T> {
    /// Creates a new uninterruptible spinlock containing the provided value.
    pub const fn new(val: T) -> UninterruptibleSpinlock<T> {
        UninterruptibleSpinlock(spin::Mutex::new(val))
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.0.get_mut()
    }

    /// Consumes this [`UninterruptibleSpinlock`], returning the underlying data.
    pub fn into_inner(self) -> T {
        self.0.into_inner()
    }

    /// Checks whether this [`UninterruptibleSpinlock`] is currently locked.
    ///
    /// # Safety
    ///
    /// This method does not actually perform any synchronization or locking, so the return value cannot be relied upon for correctness and
    /// should be treated as potentially stale immediately. This method should only be used for debugging and heuristics.
    pub fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    /// Disables interrupts and locks this [`UninterruptibleSpinlock`], returning a guard that provides access to the underlying data. The
    /// returned guard will automatically unlock this spinlock and re-enable interrupts (if applicable) once it is dropped.
    pub fn lock(&self) -> UninterruptibleSpinlockGuard<T> {
        let interrupt_disabler = InterruptDisabler::new();
        let guard = self.0.lock();

        UninterruptibleSpinlockGuard(guard, interrupt_disabler)
    }

    /// Disables interrupts and attempts to lock this [`UninterruptibleSpinlock`], returning a guard if successful. If the attempt to lock
    /// this spinlock was not successful, interrupts will remain enabled if they were enabled prior to calling this method.
    pub fn try_lock(&self) -> Option<UninterruptibleSpinlockGuard<T>> {
        let interrupt_disabler = InterruptDisabler::new();

        self.0
            .try_lock()
            .map(|guard| UninterruptibleSpinlockGuard(guard, interrupt_disabler))
    }

    /// Disables interrupts and locks this [`UninterruptibleSpinlock`], then calls the provided function with the underlying data. Once the
    /// callback returns, interrupts are re-enabled (if applicable) and the spinlock is re-locked.
    pub fn with_lock<U>(&self, f: impl FnOnce(&mut T) -> U) -> U {
        let mut lock = self.lock();
        f(lock.deref_mut())
    }

    /// Disables interrupts and attempts to lock this [`UninterruptibleSpinlock`], then calls the provided function with the underlying data
    /// if the lock was successfully obtained. If the attempt to lock this spinlock was not successful, the provided function will be called
    /// with `None`. If interrupts were enabled when calling this method and the operation was not successful, interrupts are re-enabled
    /// _before_ the provided functions is called.
    pub fn try_with_lock<U>(&self, f: impl FnOnce(Option<&mut T>) -> U) -> U {
        if let Some(mut lock) = self.try_lock() {
            f(Some(lock.deref_mut()))
        } else {
            f(None)
        }
    }
}

/// A guard that provides access to an [`UninterruptibleSpinlock`]'s internals. Releases the spinlock (and re-enables interrupts if
/// applicable) when dropped.
pub struct UninterruptibleSpinlockGuard<'a, T>(spin::MutexGuard<'a, T>, InterruptDisabler);

impl<'a, T> Deref for UninterruptibleSpinlockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T> DerefMut for UninterruptibleSpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}
