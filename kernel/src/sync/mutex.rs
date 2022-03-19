use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::sched::task::Thread;
use crate::sched::wait::ThreadWaitList;
use crate::sync::uninterruptible::InterruptDisabler;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutexLockState {
    Unlocked,
    LockedNoWaiters(NonNull<Thread>),
    LockedWithWaiters(NonNull<Thread>),
    LockedWaitersLocked(NonNull<Thread>)
}

impl MutexLockState {
    fn from_usize(val: usize) -> MutexLockState {
        if val == 0 {
            MutexLockState::Unlocked
        } else if (val & 3) == 0 {
            MutexLockState::LockedNoWaiters(NonNull::new(val as *mut _).unwrap())
        } else if (val & 3) == 1 {
            MutexLockState::LockedWithWaiters(NonNull::new((val & !1) as *mut _).unwrap())
        } else {
            MutexLockState::LockedWaitersLocked(NonNull::new((val & !1) as *mut _).unwrap())
        }
    }

    fn into_usize(self) -> usize {
        match self {
            MutexLockState::Unlocked => 0,
            MutexLockState::LockedNoWaiters(thread) => thread.as_ptr() as usize,
            MutexLockState::LockedWithWaiters(thread) => (thread.as_ptr() as usize) | 1,
            MutexLockState::LockedWaitersLocked(thread) => (thread.as_ptr() as usize) | 3
        }
    }

    fn owner(self) -> Option<NonNull<Thread>> {
        match self {
            MutexLockState::Unlocked => None,
            MutexLockState::LockedNoWaiters(owner) => Some(owner),
            MutexLockState::LockedWithWaiters(owner) => Some(owner),
            MutexLockState::LockedWaitersLocked(owner) => Some(owner)
        }
    }
}

struct MutexLock {
    state: AtomicUsize,
    wait: ThreadWaitList
}

impl MutexLock {
    const fn new() -> MutexLock {
        MutexLock {
            state: AtomicUsize::new(0),
            wait: ThreadWaitList::new()
        }
    }

    fn get_state(&self) -> MutexLockState {
        MutexLockState::from_usize(self.state.load(Ordering::Relaxed))
    }

    #[inline(always)]
    fn try_acquire_fast(&self, thread: &Pin<Arc<Thread>>) -> Result<(), usize> {
        self.state
            .compare_exchange(
                MutexLockState::Unlocked.into_usize(),
                MutexLockState::LockedNoWaiters(NonNull::from(&**thread)).into_usize(),
                Ordering::Acquire,
                Ordering::Relaxed
            )
            .map(|_| ())
    }

    #[cold]
    #[inline(never)]
    fn acquire_slow(&self, thread: &Pin<Arc<Thread>>, state: usize) {
        let mut state = MutexLockState::from_usize(state);

        loop {
            match state {
                MutexLockState::Unlocked => match self.try_acquire_fast(&thread).map_err(MutexLockState::from_usize) {
                    Ok(_) => {
                        return;
                    },
                    Err(new_state) => {
                        state = new_state;
                    }
                },
                old_state @ MutexLockState::LockedNoWaiters(owner) | old_state @ MutexLockState::LockedWithWaiters(owner) => {
                    let interrupt_disabler = InterruptDisabler::new();

                    match self
                        .state
                        .compare_exchange(
                            old_state.into_usize(),
                            MutexLockState::LockedWaitersLocked(owner).into_usize(),
                            Ordering::Acquire,
                            Ordering::Relaxed
                        )
                        .map_err(MutexLockState::from_usize)
                    {
                        Ok(_) => {
                            let suspend = self.wait.wait();

                            match self
                                .state
                                .compare_exchange(
                                    MutexLockState::LockedWaitersLocked(owner).into_usize(),
                                    MutexLockState::LockedWithWaiters(owner).into_usize(),
                                    Ordering::Acquire,
                                    Ordering::Relaxed
                                )
                                .map_err(MutexLockState::from_usize)
                            {
                                Ok(_) => {},
                                Err(state) => panic!(
                                    "Mutex state modified from {:?} to {:?} while {} was using wait list",
                                    MutexLockState::LockedWaitersLocked(owner),
                                    state,
                                    thread.debug_name()
                                )
                            }

                            drop(interrupt_disabler);
                            suspend.suspend();

                            while self.get_state().owner() == Some(owner) {
                                core::hint::spin_loop();
                            }

                            assert_eq!(Some(NonNull::from(&**thread)), self.get_state().owner());
                            return;
                        },
                        Err(new_state) => {
                            state = new_state;
                        }
                    }
                },
                MutexLockState::LockedWaitersLocked(_) => {
                    while matches!(state, MutexLockState::LockedWaitersLocked(_)) {
                        core::hint::spin_loop();
                        state = self.get_state();
                    }
                },
            }
        }
    }

    fn acquire(&self) {
        let thread = Thread::current();

        match self.try_acquire_fast(&thread) {
            Ok(()) => {},
            Err(state) => {
                self.acquire_slow(&thread, state);
            }
        }
    }

    fn try_acquire(&self) -> bool {
        self.try_acquire_fast(&Thread::current()).is_ok()
    }

    #[cold]
    #[inline(never)]
    fn release_slow(&self, thread: &Pin<Arc<Thread>>, state: usize) {
        let mut state = MutexLockState::from_usize(state);

        loop {
            match state {
                MutexLockState::LockedWithWaiters(lock_thread) if lock_thread == NonNull::from(&**thread) => {
                    let _interrupt_disabler = InterruptDisabler::new();

                    match self
                        .state
                        .compare_exchange(
                            MutexLockState::LockedWithWaiters(lock_thread).into_usize(),
                            MutexLockState::LockedWaitersLocked(lock_thread).into_usize(),
                            Ordering::Acquire,
                            Ordering::Relaxed
                        )
                        .map_err(MutexLockState::from_usize)
                    {
                        Ok(_) => {
                            let new_state = if let Some(next_owner) = self.wait.wake_one() {
                                MutexLockState::LockedWithWaiters(NonNull::from(&*next_owner))
                            } else {
                                MutexLockState::Unlocked
                            };

                            self.state.store(new_state.into_usize(), Ordering::Release);
                            return;
                        },
                        Err(new_state) => {
                            state = new_state;
                        }
                    }
                },
                MutexLockState::LockedWaitersLocked(lock_thread) if lock_thread == NonNull::from(&**thread) => {
                    while state == MutexLockState::LockedWaitersLocked(lock_thread) {
                        core::hint::spin_loop();
                        state = self.get_state();
                    }
                },
                state => {
                    panic!(
                        "Saw unexpected mutex state {:?} while unlocking mutex from {}",
                        state,
                        thread.debug_name()
                    );
                }
            }
        }
    }

    fn release(&self) {
        let thread = Thread::current();

        match self.state.compare_exchange(
            MutexLockState::LockedNoWaiters(NonNull::from(&*thread)).into_usize(),
            MutexLockState::Unlocked.into_usize(),
            Ordering::Release,
            Ordering::Relaxed
        ) {
            Ok(_) => {},
            Err(state) => {
                self.release_slow(&thread, state);
            }
        }
    }
}

pub struct Mutex<T: Send> {
    data: UnsafeCell<T>,
    lock: MutexLock
}

impl<T: Send> Mutex<T> {
    pub const fn new(val: T) -> Mutex<T> {
        Mutex {
            data: UnsafeCell::new(val),
            lock: MutexLock::new()
        }
    }

    pub fn lock(&self) -> MutexGuard<T> {
        self.lock.acquire();
        MutexGuard {
            data: unsafe { &mut *self.data.get() },
            lock: &self.lock
        }
    }

    pub fn try_lock(&self) -> Option<MutexGuard<T>> {
        if self.lock.try_acquire() {
            Some(MutexGuard {
                data: unsafe { &mut *self.data.get() },
                lock: &self.lock
            })
        } else {
            None
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }

    pub fn is_locked(&self) -> bool {
        match self.lock.get_state() {
            MutexLockState::Unlocked => false,
            _ => true
        }
    }
}

unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

pub struct MutexGuard<'a, T: Send> {
    data: &'a mut T,
    lock: &'a MutexLock
}

impl<'a, T: Send> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<'a, T: Send> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

impl<'a, T: Send> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.release();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::sched::task::{Process, ThreadState};

    #[test_case]
    fn test_basics() {
        let mut mutex = Mutex::new(0);

        assert_eq!(0, *mutex.get_mut());
        assert!(!mutex.is_locked());

        {
            let mut guard = mutex.lock();

            assert_eq!(0, *guard);
            assert!(mutex.is_locked());

            *guard = 1;
        }

        assert_eq!(1, *mutex.get_mut());
        assert!(!mutex.is_locked());
    }

    #[test_case]
    fn test_two_threads() {
        let mutex = Mutex::new(0);
        let thread = unsafe {
            Process::kernel().lock().create_kernel_thread_unchecked(
                || {
                    let mut guard = mutex.lock();
                    *guard = 1;
                    Thread::yield_current();
                    Thread::yield_current();
                    Thread::yield_current();

                    assert_eq!(1, *guard);
                    *guard = 2;
                    drop(guard);
                    Thread::yield_current();

                    assert!(!mutex.is_locked());
                    assert_eq!(3, *mutex.lock());
                },
                4096
            )
        };

        thread.lock().wake();

        assert!(!mutex.is_locked());
        Thread::yield_current();
        assert!(mutex.is_locked());

        let mut guard = mutex.lock();
        assert_eq!(2, *guard);
        *guard = 3;
        drop(guard);

        Thread::yield_current();
        assert!(!mutex.is_locked());
        assert_eq!(ThreadState::Dead, *thread.lock().state());
    }
}
