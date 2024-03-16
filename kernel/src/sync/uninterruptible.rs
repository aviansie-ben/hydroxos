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
//!
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

use alloc::fmt;
use core::cell::Cell;
use core::mem;
use core::ops::{Deref, DerefMut};
use core::ptr;

use x86_64::instructions::interrupts;

use crate::sched;
use crate::util::SharedUnsafeCell;

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

    /// Get whether interrupts were enabled when the first InterruptDisabler held on the current CPU core was created.
    ///
    /// # Panics
    ///
    /// This method will panic if no InterruptDisablers are held on the current CPU core. The caller should ensure that
    /// [`InterruptDisabler::num_held`] is non-zero before calling this method.
    pub fn was_enabled() -> bool {
        assert!(InterruptDisabler::num_held() > 0);
        INTERRUPT_DISABLER_STATE.get().1
    }

    /// Forces interrupts to remain disabled on the current CPU core when the last InterruptDisabler is dropped.
    pub fn force_remain_disabled() {
        INTERRUPT_DISABLER_STATE.set((InterruptDisabler::num_held(), false));
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
            sched::run_soft_interrupts();
            assert!(INTERRUPT_DISABLER_STATE.get().0 == 0);

            interrupts::enable();
        };
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "spinlock_tracking")] {
        mod tracking {
            use core::cell::{Cell, UnsafeCell};
            use core::ptr;

            use itertools::Itertools;

            use super::RawSpinlock;

            const MAX_HELD_LOCKS: usize = 64;

            #[thread_local]
            static HELD_LOCKS: UnsafeCell<[*const RawSpinlock; MAX_HELD_LOCKS]> = UnsafeCell::new([ptr::null(); MAX_HELD_LOCKS]);

            #[thread_local]
            static HELD_LOCKS_LEN: Cell<usize> = Cell::new(0);

            pub unsafe fn held_spinlocks() -> &'static [*const RawSpinlock] {
                unsafe { &(*HELD_LOCKS.get())[..HELD_LOCKS_LEN.get()] }
            }

            pub fn check_spinlock_for_deadlock(lock: *const RawSpinlock) {
                if unsafe { (*HELD_LOCKS.get())[..HELD_LOCKS_LEN.get()].contains(&lock) } {
                    panic!("Attempt to acquire spinlock {:?} already held by current core", lock);
                }
            }

            pub fn push_spinlock(lock: *const RawSpinlock) {
                if HELD_LOCKS_LEN.get() == MAX_HELD_LOCKS {
                    panic!("Acquired too many spinlocks!");
                }

                unsafe {
                    (*HELD_LOCKS.get())[HELD_LOCKS_LEN.get()] = lock;
                }
                HELD_LOCKS_LEN.set(HELD_LOCKS_LEN.get() + 1);
            }

            pub fn pop_spinlock(lock: *const RawSpinlock) {
                let held_locks = unsafe { &mut (*HELD_LOCKS.get())[..HELD_LOCKS_LEN.get()] };

                if let Some((idx, _)) = held_locks.iter().find_position(|&&l| l == lock) {
                    held_locks.copy_within((idx + 1).., idx);
                    HELD_LOCKS_LEN.set(HELD_LOCKS_LEN.get() - 1);
                } else {
                    panic!("Attempt to release spinlock {:?} not held by current core", lock);
                }
            }
        }
    } else {
        mod tracking {
            static EMPTY_HELD: [*const RawSpinlock; 0] = [];

            pub unsafe fn held_spinlocks() -> &'static [*const RawSpinlock] { &EMPTY_HELD[..] }
            pub fn check_spinlock_for_deadlock(_: *const RawSpinlock) {}
            pub fn push_spinlock(_: *const RawSpinlock) {}
            pub fn pop_spinlock(_: *const RawSpinlock) {}
        }
    }
}

/// A raw spinlock implementation that does not guard any actual data.
///
/// Note that this implementation **does not** automatically disable interrupts when it is held, so
/// it should not be used to protect access to any data that may be needed from within an interrupt
/// handler unless a separate [`InterruptDisabler`] is used.
pub struct RawSpinlock(spin::Mutex<()>);

impl RawSpinlock {
    /// Creates a new unlocked spinlock.
    pub const fn new() -> RawSpinlock {
        RawSpinlock(spin::Mutex::new(()))
    }

    /// Gets a list of spinlocks held by the current CPU core for debugging purposes.
    ///
    /// This method may not always return the expected spinlocks, as it is possible to compile
    /// without support for spinlock tracking. This method should **never** be relied on for
    /// correctness and is provided only for debugging purposes.
    ///
    /// # Safety
    ///
    /// The returned slice is valid only as long as [`RawSpinlockGuard`] is dropped which was live
    /// at the time this method was called. It is, however, acceptable to lock new spinlocks and
    /// drop their guards between calling this method and reading the returned slice.
    pub unsafe fn held() -> &'static [*const RawSpinlock] {
        tracking::held_spinlocks()
    }

    /// Locks this spinlock and returns a guard that will automatically unlock it when dropped.
    pub fn lock(&self) -> RawSpinlockGuard {
        let guard = if let Some(guard) = self.0.try_lock() {
            guard
        } else {
            tracking::check_spinlock_for_deadlock(self);
            self.0.lock()
        };

        tracking::push_spinlock(self);
        spin::MutexGuard::leak(guard);
        RawSpinlockGuard(self)
    }

    /// Attempts to lock this spinlock if it is currently unlocked and returns a guard that will
    /// automatically unlock it when dropped if successful.
    pub fn try_lock(&self) -> Option<RawSpinlockGuard> {
        self.0.try_lock().map(|guard| {
            tracking::push_spinlock(self);
            spin::MutexGuard::leak(guard);
            RawSpinlockGuard(self)
        })
    }

    /// Checks whether this spinlock is currently locked.
    ///
    /// This method does not actually perform any synchronization or locking, so the return value cannot be relied upon for correctness and
    /// should be treated as potentially stale immediately. This method should only be used for debugging and heuristics.
    pub fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    /// Checks whether this spinlock is currently being locked by the provided guard.
    pub fn is_guarded_by(&self, guard: &RawSpinlockGuard) -> bool {
        ptr::eq(self, guard.0)
    }

    /// Forcibly unlocks this spinlock without checking that the current core owns it.
    ///
    /// This is generally a _very bad idea_ except in post-mortem debugging scenarios, since the
    /// original guard that locked this spinlock will persist.
    pub unsafe fn force_unlock(&self) {
        self.0.force_unlock();
    }
}

impl fmt::Debug for RawSpinlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "RawSpinlock@{:p}({})",
            self,
            if self.is_locked() { "<locked>" } else { "<unlocked>" }
        )
    }
}

/// A guard which will automatically unlock a locked [`RawSpinlock`] when dropped.
pub struct RawSpinlockGuard<'a>(&'a RawSpinlock);

impl<'a> RawSpinlockGuard<'a> {
    /// Leaks this spinlock guard, leaving the referenced spinlock permanently locked.
    ///
    /// Note that unlike simply leaking the guard through other means, this method will properly
    /// free internal memory used to track which spinlocks the current core owns. Leaking the guard
    /// through other means does not free this memory, and so can result in a kernel panic if done
    /// enough times.
    pub fn leak(self) {
        tracking::pop_spinlock(self.0);
        mem::forget(self);
    }

    /// Gets a reference to the spinlock locked by this guard.
    pub fn get_lock(&self) -> &'a RawSpinlock {
        self.0
    }
}

impl<'a> !Send for RawSpinlockGuard<'a> {}
impl<'a> !Sync for RawSpinlockGuard<'a> {}

impl<'a> fmt::Debug for RawSpinlockGuard<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RawSpinlockGuard@{:p}", self.0)
    }
}

impl<'a> Drop for RawSpinlockGuard<'a> {
    fn drop(&mut self) {
        unsafe {
            tracking::pop_spinlock(self.0);
            self.0.force_unlock();
        }
    }
}

/// A spinlock that keeps interrupts disabled on the local CPU core while it is locked.
pub struct UninterruptibleSpinlock<T: ?Sized>(RawSpinlock, SharedUnsafeCell<T>);

impl<T> UninterruptibleSpinlock<T> {
    /// Creates a new uninterruptible spinlock containing the provided value.
    pub const fn new(val: T) -> UninterruptibleSpinlock<T> {
        UninterruptibleSpinlock(RawSpinlock::new(), SharedUnsafeCell::new(val))
    }

    /// Consumes this [`UninterruptibleSpinlock`], returning the underlying data.
    pub fn into_inner(self) -> T {
        let val = unsafe { ptr::read(self.1.get()) };
        mem::forget(self);

        val
    }
}

impl<T: ?Sized> UninterruptibleSpinlock<T> {
    /// Gets a reference to the [`RawSpinlock`] underpinning this spinlock.
    pub fn raw(&self) -> &RawSpinlock {
        &self.0
    }

    /// Gets a mutable reference to the contents of this [`UninterruptibleSpinlock`] given a
    /// mutable reference to the spinlock itself.
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.1.get() }
    }

    /// Checks whether this [`UninterruptibleSpinlock`] is currently being locked by the provided
    /// guard.
    pub fn is_guarded_by<U: ?Sized>(&self, guard: &UninterruptibleSpinlockGuard<U>) -> bool {
        self.0.is_guarded_by(&guard.0)
    }

    /// Checks whether this [`UninterruptibleSpinlock`] is currently being locked by the provided
    /// guard.
    pub fn is_read_guarded_by<U: ?Sized>(&self, guard: &UninterruptibleSpinlockReadGuard<U>) -> bool {
        self.0.is_guarded_by(&guard.0)
    }

    /// Checks whether this [`UninterruptibleSpinlock`] is currently locked.
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

        UninterruptibleSpinlockGuard(guard, unsafe { &mut *self.1.get() }, interrupt_disabler)
    }

    /// Disables interrupts and attempts to lock this [`UninterruptibleSpinlock`], returning a guard if successful. If the attempt to lock
    /// this spinlock was not successful, interrupts will remain enabled if they were enabled prior to calling this method.
    pub fn try_lock(&self) -> Option<UninterruptibleSpinlockGuard<T>> {
        let interrupt_disabler = InterruptDisabler::new();

        self.0
            .try_lock()
            .map(|guard| UninterruptibleSpinlockGuard(guard, unsafe { &mut *self.1.get() }, interrupt_disabler))
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for UninterruptibleSpinlock<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "UninterruptibleSpinlock@{:p}(", self)?;

        match self.try_lock() {
            Some(guard) => {
                write!(f, "{:?}", &*guard)?;
            },
            None => {
                write!(f, "<locked>")?;
            }
        }

        write!(f, ")")
    }
}

/// A guard that provides access to an [`UninterruptibleSpinlock`]'s internals. Releases the spinlock (and re-enables interrupts if
/// applicable) when dropped.
pub struct UninterruptibleSpinlockGuard<'a, T: ?Sized>(RawSpinlockGuard<'a>, &'a mut T, InterruptDisabler);

impl<'a, T: ?Sized + 'a> UninterruptibleSpinlockGuard<'a, T> {
    /// Maps the data referenced by this guard, returning a guard that guards the same spinlock but
    /// returns the newly mapped data when dereferenced.
    pub fn map<U: ?Sized, Guard>(guard: Guard, f: impl FnOnce(&mut T) -> &mut U) -> UninterruptibleSpinlockGuard<'a, U>
    where
        Self: From<Guard>
    {
        let Self(guard, data, interrupt_disabler) = Self::from(guard);

        UninterruptibleSpinlockGuard(guard, f(data), interrupt_disabler)
    }
}

impl<'a, T: ?Sized> Deref for UninterruptibleSpinlockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.1
    }
}

impl<'a, T: ?Sized> DerefMut for UninterruptibleSpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.1
    }
}

impl<'a, T: ?Sized + fmt::Debug> fmt::Debug for UninterruptibleSpinlockGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "UninterruptibleSpinlockGuard@{:p}({:?})", self.0.get_lock(), self.1)
    }
}

/// A guard that provides read-only access to an [`UninterruptibleSpinlock`]'s internals. Releases
/// the spinlock (and re-enables interrupts if applicable) when dropped.
pub struct UninterruptibleSpinlockReadGuard<'a, T: ?Sized>(RawSpinlockGuard<'a>, &'a T, InterruptDisabler);

impl<'a, T: ?Sized + 'a> UninterruptibleSpinlockReadGuard<'a, T> {
    /// Maps the data referenced by this guard, returning a guard that guards the same spinlock but
    /// returns the newly mapped data when dereferenced.
    pub fn map<U: ?Sized, Guard>(guard: Guard, f: impl FnOnce(&T) -> &U) -> UninterruptibleSpinlockReadGuard<'a, U>
    where
        Self: From<Guard>
    {
        let Self(guard, data, interrupt_disabler) = Self::from(guard);

        UninterruptibleSpinlockReadGuard(guard, f(data), interrupt_disabler)
    }
}

impl<'a, T: ?Sized> From<UninterruptibleSpinlockGuard<'a, T>> for UninterruptibleSpinlockReadGuard<'a, T> {
    fn from(value: UninterruptibleSpinlockGuard<'a, T>) -> Self {
        let UninterruptibleSpinlockGuard(guard, data, interrupt_disabler) = value;

        Self(guard, data, interrupt_disabler)
    }
}

impl<'a, T: ?Sized> Deref for UninterruptibleSpinlockReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.1
    }
}

impl<'a, T: ?Sized + fmt::Debug> fmt::Debug for UninterruptibleSpinlockReadGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "UninterruptibleSpinlockReadGuard@{:p}({:?})", self.0.get_lock(), self.1)
    }
}

#[cfg(test)]
mod test {
    use cfg_if::cfg_if;

    use super::*;
    #[allow(unused_imports)]
    use crate::test_util::skip;

    #[test_case]
    fn test_interrupt_disabler() {
        assert_eq!(0, InterruptDisabler::num_held());
        assert!(interrupts::are_enabled());

        {
            let _disabler = InterruptDisabler::new();

            assert!(!interrupts::are_enabled());
            assert_eq!(1, InterruptDisabler::num_held());
            assert!(InterruptDisabler::was_enabled());
        }

        assert_eq!(0, InterruptDisabler::num_held());
        assert!(interrupts::are_enabled());
    }

    #[test_case]
    fn test_interrupt_disabler_nested() {
        assert_eq!(0, InterruptDisabler::num_held());
        assert!(interrupts::are_enabled());

        {
            let _disabler_1 = InterruptDisabler::new();

            {
                let _disabler_2 = InterruptDisabler::new();

                assert!(!interrupts::are_enabled());
                assert_eq!(2, InterruptDisabler::num_held());
                assert!(InterruptDisabler::was_enabled());
            }

            assert!(!interrupts::are_enabled());
            assert_eq!(1, InterruptDisabler::num_held());
            assert!(InterruptDisabler::was_enabled());
        }

        assert_eq!(0, InterruptDisabler::num_held());
        assert!(interrupts::are_enabled());
    }

    #[test_case]
    fn test_interrupt_disabler_keep_disable() {
        assert_eq!(0, InterruptDisabler::num_held());
        assert!(interrupts::are_enabled());

        interrupts::disable();

        {
            let _disabler = InterruptDisabler::new();

            assert!(!interrupts::are_enabled());
            assert_eq!(1, InterruptDisabler::num_held());
            assert!(!InterruptDisabler::was_enabled());
        }

        assert_eq!(0, InterruptDisabler::num_held());
        assert!(!interrupts::are_enabled());

        interrupts::enable();
    }

    #[test_case]
    fn test_interrupt_disabler_force_disable() {
        assert_eq!(0, InterruptDisabler::num_held());
        assert!(interrupts::are_enabled());

        {
            let _disabler = InterruptDisabler::new();

            InterruptDisabler::force_remain_disabled();

            assert!(!interrupts::are_enabled());
            assert_eq!(1, InterruptDisabler::num_held());
            assert!(!InterruptDisabler::was_enabled());
        }

        assert_eq!(0, InterruptDisabler::num_held());
        assert!(!interrupts::are_enabled());

        interrupts::enable();
    }

    #[test_case]
    fn test_spinlock_tracking() {
        cfg_if! {
            if #[cfg(feature = "spinlock_tracking")] {
                let lock1 = UninterruptibleSpinlock::new(());
                let lock2 = UninterruptibleSpinlock::new(());
                let lock3 = UninterruptibleSpinlock::new(());

                assert_eq!(
                    &[] as &[*const RawSpinlock],
                    unsafe { super::tracking::held_spinlocks() }
                );

                let lock1_guard = lock1.lock();
                let lock2_guard = lock2.lock();
                let lock3_guard = lock3.lock();

                assert_eq!(
                    &[
                        lock1.raw() as *const _,
                        lock2.raw() as *const _,
                        lock3.raw() as *const _
                    ],
                    unsafe { super::tracking::held_spinlocks() }
                );

                drop(lock2_guard);

                assert_eq!(
                    &[
                        lock1.raw() as *const _,
                        lock3.raw() as *const _
                    ],
                    unsafe { super::tracking::held_spinlocks() }
                );

                drop(lock3_guard);

                assert_eq!(
                    &[
                        lock1.raw() as *const _
                    ],
                    unsafe { super::tracking::held_spinlocks() }
                );

                drop(lock1_guard);

                assert_eq!(
                    &[] as &[*const RawSpinlock],
                    unsafe { super::tracking::held_spinlocks() }
                );
            } else {
                skip("spinlock tracking disabled");
            }
        }
    }

    #[test_case]
    fn test_spinlock_is_guarded_by() {
        let lock1 = UninterruptibleSpinlock::new(());
        let lock2 = UninterruptibleSpinlock::new(());

        let lock1_guard = lock1.lock();
        let lock2_guard = lock2.lock();

        assert!(lock1.is_guarded_by(&lock1_guard));
        assert!(!lock1.is_guarded_by(&lock2_guard));

        assert!(!lock2.is_guarded_by(&lock1_guard));
        assert!(lock2.is_guarded_by(&lock2_guard));
    }

    #[test_case]
    fn test_spinlock_guard_map() {
        let mut lock = UninterruptibleSpinlock::new((0, 0));
        let p0 = &lock.get_mut().0 as *const _;
        let p1 = &lock.get_mut().1 as *const _;

        let guard = UninterruptibleSpinlockGuard::map(lock.lock(), |r| &mut r.0);

        assert!(lock.is_locked());
        assert!(lock.is_guarded_by(&guard));
        drop(guard);
        assert!(!lock.is_locked());

        let guard = UninterruptibleSpinlockReadGuard::map(lock.lock(), |r| &r.0);

        assert!(lock.is_locked());
        assert!(lock.is_read_guarded_by(&guard));
        drop(guard);
        assert!(!lock.is_locked());

        assert_eq!(p0, &*UninterruptibleSpinlockGuard::map(lock.lock(), |r| &mut r.0) as *const _);
        assert_eq!(p1, &*UninterruptibleSpinlockGuard::map(lock.lock(), |r| &mut r.1) as *const _);

        assert_eq!(p0, &*UninterruptibleSpinlockReadGuard::map(lock.lock(), |r| &r.0) as *const _);
        assert_eq!(p1, &*UninterruptibleSpinlockReadGuard::map(lock.lock(), |r| &r.1) as *const _);
    }
}
