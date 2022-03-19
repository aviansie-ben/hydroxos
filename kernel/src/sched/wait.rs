//! Low-level synchronization primitives for inter-thread synchronization.

use alloc::sync::Arc;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::pin::Pin;
use core::ptr;

use super::task::{Thread, ThreadLock, ThreadState};
use crate::sync::UninterruptibleSpinlock;

/// State information for a thread which is waiting on a wait list.
#[derive(Debug)]
pub(super) struct ThreadWaitState {
    prev: *const Thread,
    next: *const Thread,
    valid: bool
}

unsafe impl Send for ThreadWaitState {}

impl ThreadWaitState {
    pub fn new() -> ThreadWaitState {
        ThreadWaitState {
            prev: ptr::null(),
            next: ptr::null(),
            valid: false
        }
    }
}

struct ThreadWaitListInternal {
    head: *const Thread,
    tail: *const Thread
}

unsafe impl Send for ThreadWaitListInternal {}

impl ThreadWaitListInternal {
    fn dequeue(&mut self) -> Option<Pin<Arc<Thread>>> {
        if !self.head.is_null() {
            // SAFETY: When the thread was enqueued, into_raw was called on it exactly one time.
            let thread = unsafe { Thread::from_raw(self.head) };

            // SAFETY: The wait list effectively has a mutable borrow of the wait states of all threads that appear on it.
            unsafe {
                assert!((*(*thread).wait_state()).valid);
                (*(*thread).wait_state()).valid = false;

                self.head = (*(*thread).wait_state()).next;
                if self.head.is_null() {
                    self.tail = ptr::null();
                } else {
                    (*(*self.head).wait_state()).prev = ptr::null();
                }
            }

            Some(thread)
        } else {
            None
        }
    }

    unsafe fn enqueue(&mut self, thread: Pin<Arc<Thread>>) {
        assert!(!(*thread.wait_state()).valid);

        (*thread.wait_state()).prev = self.tail;
        (*thread.wait_state()).next = ptr::null();
        (*thread.wait_state()).valid = true;

        if self.tail.is_null() {
            self.head = &*thread;
        } else {
            (*(*self.tail).wait_state()).next = &*thread;
        };
        self.tail = thread.into_raw();
    }
}

/// A struct representing that the thread has been placed on a wait list and needs to be suspended.
///
/// This struct is responsible for making sure that the spinlock for the current thread is held between the time that the current thread is
/// placed on a wait list and when its state is actually saved by performing a context switch.
///
/// # Panics
///
/// Causing a value of this type to be dropped without running [`ThreadWait::suspend`] will cause a panic. This is because releasing the
/// spinlock on the current thread after adding the thread to a wait list but before its state has been saved can cause undefined behaviour
/// due to other cores seeing inconsistent state information in the thread.
pub struct ThreadWait<'a>(MaybeUninit<ThreadLock<'static>>, PhantomData<&'a ThreadWaitList>);

impl<'a> ThreadWait<'a> {
    /// Suspends the current thread and consumes this guard.
    ///
    /// # Panics
    ///
    /// This method will panic if any [`InterruptDisabler`](crate::sync::uninterruptible::InterruptDisabler) values currently exist on this
    /// thread, aside from the single one owned by the thread lock that is part of this object itself. Context switching while an
    /// uninterruptible lock guard is held could result in a deadlock due to the new thread trying to acquire a lock that was held prior to
    /// a context switch.
    pub fn suspend(self) {
        unsafe {
            let this = ManuallyDrop::new(self);
            Thread::suspend_current(ptr::read(this.0.as_ptr()));
        };
    }
}

impl<'a> Drop for ThreadWait<'a> {
    fn drop(&mut self) {
        panic!("Missing call to ThreadWait::suspend after calling ThreadWaitList::wait");
    }
}

/// A wait list onto which threads can enqueue themselves to be woken up later.
pub struct ThreadWaitList {
    internal: UninterruptibleSpinlock<ThreadWaitListInternal>
}

impl !Unpin for ThreadWaitList {}

impl ThreadWaitList {
    /// Creates an empty wait list.
    pub const fn new() -> ThreadWaitList {
        ThreadWaitList {
            internal: UninterruptibleSpinlock::new(ThreadWaitListInternal {
                head: ptr::null(),
                tail: ptr::null()
            })
        }
    }

    /// Adds the current thread to the wait list and puts it into the waiting state. Returns a [`ThreadWait`] that should be used to suspend
    /// the current thread by calling [`ThreadWait::suspend`] after releasing any held spinlocks.
    ///
    /// # Lock Ordering
    ///
    /// This method should not be called while any scheduler locks, such as thread and process locks, are held. Doing so may result in a
    /// deadlock occurring. The returned [`ThreadWait`] will hold the spinlock for the current thread until [`ThreadWait::suspend`] is
    /// called.
    ///
    /// # Panics
    ///
    /// This method cannot suspend a thread from the context of an asynchronous hardware interrupt and will panic if it is called from an
    /// asynchronous interrupt handler.
    #[must_use]
    pub fn wait(&self) -> ThreadWait {
        unsafe {
            // SAFETY: This thread reference never leaves the current thread. Since this references the current thread, it must continue to
            //         exist while this thread is still executing, so extending its lifetime like this is safe.
            let mut thread = (*(&*Thread::current() as *const Thread)).lock();
            let mut internal = self.internal.lock();

            assert!(matches!(*thread.state(), ThreadState::Running));
            *thread.state_mut() = ThreadState::Waiting(&*self);

            // SAFETY: The only way for the caller to release the thread lock at this point would be to either call ThreadWait::suspend or
            //         drop the returned ThreadWait, which will unconditionally panic. If the returned ThreadWait is leaked, then the thread
            //         is never unlocked and the improper state updates can never be observed. Obviously, this is undesirable but does not
            //         have any implications for safety guarantees.
            internal.enqueue(thread.thread().as_arc());
            ThreadWait(MaybeUninit::new(thread), PhantomData)
        }
    }

    unsafe fn try_wake(&self, mut thread: ThreadLock) -> bool {
        match *thread.state() {
            ThreadState::Dead => false,
            ThreadState::Waiting(list) if list == self => {
                *thread.state_mut() = ThreadState::Suspended;
                thread.wake();
                true
            },
            ref state => {
                panic!(
                    "Thread {} is in unexpected state {:?} after dequeueing from wait list",
                    thread.thread().debug_name(),
                    state
                );
            }
        }
    }

    /// Removes a single thread from the wait list and puts it in the ready state. Returns `true` if there was a thread on the wait list
    /// that was awoken and `false` otherwise.
    ///
    /// # Lock Ordering
    ///
    /// This method should not be called while any scheduler locks, such as thread and process locks, are held. Doing so may result in a
    /// deadlock occurring.
    pub fn wake_one(&self) -> bool {
        loop {
            break if let Some(thread) = self.internal.lock().dequeue() {
                // SAFETY: A waiting -> ready transition is safe since the event the thread was waiting on has now occurred.
                unsafe {
                    if !self.try_wake(thread.lock()) {
                        continue;
                    }
                };

                true
            } else {
                false
            };
        }
    }

    /// Removes all threads from the wait list and puts them in the ready state. Returns the number of threads awoken by this call.
    ///
    /// # Lock Ordering
    ///
    /// This method should not be called while any scheduler locks, such as thread and process locks, are held. Doing so may result in a
    /// deadlock occurring.
    pub fn wake_all(&self) -> usize {
        let mut num_woken = 0;

        while let Some(thread) = self.internal.lock().dequeue() {
            // SAFETY: A waiting -> ready transition (via suspended) is safe since the event the thread was waiting on has now occurred.
            unsafe {
                if self.try_wake(thread.lock()) {
                    num_woken += 1;
                }
            };
        }

        num_woken
    }
}

impl Drop for ThreadWaitList {
    fn drop(&mut self) {
        if !self.internal.try_lock().unwrap().head.is_null() {
            panic!("Attempt to drop non-empty ThreadWaitList");
        };
    }
}

#[cfg(test)]
mod test {
    use alloc::boxed::Box;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    use super::super::task::{Process, Thread};
    use super::*;

    #[test_case]
    fn test_wake_one() {
        let flag = AtomicBool::new(false);
        let waitlist = Box::pin(ThreadWaitList::new());

        let thread_fn = || {
            waitlist.as_ref().wait().suspend();
            flag.store(true, Ordering::Relaxed);
        };

        let thread = unsafe { Process::kernel().lock().create_kernel_thread_unchecked(thread_fn, 4096) };
        thread.lock().wake();

        Thread::yield_current();
        Thread::yield_current();
        assert!(!flag.load(Ordering::Relaxed));
        waitlist.wake_one();
        Thread::yield_current();
        assert!(flag.load(Ordering::Relaxed));
        assert!(matches!(*thread.lock().state(), ThreadState::Dead));
    }

    #[test_case]
    fn test_wake_one_order() {
        let val = AtomicI32::new(0);
        let waitlist = Box::pin(ThreadWaitList::new());

        let thread_fn_1 = || {
            waitlist.as_ref().wait().suspend();
            val.store(1, Ordering::Relaxed);
        };
        let thread_1 = unsafe { Process::kernel().lock().create_kernel_thread_unchecked(thread_fn_1, 4096) };
        thread_1.lock().wake();
        Thread::yield_current();

        let thread_fn_2 = || {
            waitlist.as_ref().wait().suspend();
            val.store(2, Ordering::Relaxed);
        };
        let thread_2 = unsafe { Process::kernel().lock().create_kernel_thread_unchecked(thread_fn_2, 4096) };
        thread_2.lock().wake();
        Thread::yield_current();

        assert_eq!(0, val.load(Ordering::Relaxed));

        waitlist.wake_one();
        Thread::yield_current();
        assert_eq!(1, val.load(Ordering::Relaxed));
        assert!(matches!(*thread_1.lock().state(), ThreadState::Dead));

        waitlist.wake_one();
        Thread::yield_current();
        assert_eq!(2, val.load(Ordering::Relaxed));
        assert!(matches!(*thread_2.lock().state(), ThreadState::Dead));
    }

    #[test_case]
    fn test_wake_all() {
        let val = AtomicI32::new(0);
        let waitlist = Box::pin(ThreadWaitList::new());

        let thread_fn_1 = || {
            waitlist.as_ref().wait().suspend();
            val.fetch_add(1, Ordering::Relaxed);
        };
        let thread_1 = unsafe { Process::kernel().lock().create_kernel_thread_unchecked(thread_fn_1, 4096) };
        thread_1.lock().wake();
        Thread::yield_current();

        let thread_fn_2 = || {
            waitlist.as_ref().wait().suspend();
            val.fetch_add(1, Ordering::Relaxed);
        };
        let thread_2 = unsafe { Process::kernel().lock().create_kernel_thread_unchecked(thread_fn_2, 4096) };
        thread_2.lock().wake();
        Thread::yield_current();

        assert_eq!(0, val.load(Ordering::Relaxed));

        waitlist.wake_all();
        Thread::yield_current();
        assert_eq!(2, val.load(Ordering::Relaxed));
        assert!(matches!(*thread_1.lock().state(), ThreadState::Dead));
        assert!(matches!(*thread_2.lock().state(), ThreadState::Dead));
    }
}
