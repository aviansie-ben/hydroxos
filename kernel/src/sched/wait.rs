//! Low-level synchronization primitives for inter-thread synchronization.

use core::mem::{ManuallyDrop, MaybeUninit};
use core::pin::Pin;
use core::ptr;

use super::task::{Thread, ThreadLock, ThreadState};
use crate::util::InterruptDisableSpinlock;

/// State information for a thread which is waiting on a wait list.
#[derive(Debug)]
pub struct ThreadWaitState {
    list: *const ThreadWaitList,
    prev: *const Thread,
    next: *const Thread
}

unsafe impl Send for ThreadWaitState {}

struct ThreadWaitListInternal {
    head: *const Thread,
    tail: *const Thread
}

unsafe impl Send for ThreadWaitListInternal {}

impl ThreadWaitListInternal {
    fn dequeue(&mut self, list: *const ThreadWaitList) -> Option<ThreadLock> {
        if !self.head.is_null() {
            // SAFETY: So long as the wait list is in a valid state, head should either point at a valid thread or be null
            let thread_lock = unsafe { (*self.head).lock() };

            if let ThreadState::Waiting(ref state) = thread_lock.state() {
                if state.list != list {
                    panic!("{} present in wait list {:p}, but is in state {:?}", thread_lock.thread().debug_name(), self, thread_lock.state());
                };

                self.head = state.next;
                if self.head.is_null() {
                    self.tail = ptr::null();
                };
            } else {
                panic!("{} present in wait list {:p}, but is in state {:?}", thread_lock.thread().debug_name(), self, thread_lock.state());
            };

            Some(thread_lock)
        } else {
            None
        }
    }

    unsafe fn enqueue(&mut self, thread: &mut ThreadLock, list: *const ThreadWaitList) {
        *thread.state_mut() = ThreadState::Waiting(ThreadWaitState {
            list,
            prev: self.tail,
            next: ptr::null()
        });

        if self.head.is_null() {
            self.head = thread.thread();
        };
        self.tail = thread.thread();
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
pub struct ThreadWait(MaybeUninit<ThreadLock<'static>>);

impl ThreadWait {
    /// Suspends the current thread and consumes this guard.
    ///
    /// # Panics
    ///
    /// This method will panic if any [`InterruptDisabler`](crate::util::InterruptDisabler) values currently exist on this thread, aside
    /// from the single one owned by the thread lock that is part of this object itself. Context switching while an interrupt-disabling lock
    /// guard is held could result in a deadlock due to the new thread trying to acquire a lock that was held prior to a context switch.
    pub fn suspend(self) {
        unsafe {
            let this = ManuallyDrop::new(self);
            Thread::suspend_current(ptr::read(this.0.as_ptr()));
        };
    }
}

impl Drop for ThreadWait {
    fn drop(&mut self) {
        panic!("Missing call to ThreadWait::suspend after calling ThreadWaitList::wait");
    }
}

/// A wait list onto which threads can enqueue themselves to be woken up later.
pub struct ThreadWaitList {
    internal: InterruptDisableSpinlock<ThreadWaitListInternal>
}

impl !Unpin for ThreadWaitList {}

impl ThreadWaitList {
    /// Creates an empty wait list.
    pub const fn new() -> ThreadWaitList {
        ThreadWaitList {
            internal: InterruptDisableSpinlock::new(ThreadWaitListInternal {
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
    pub fn wait(self: Pin<&Self>) -> ThreadWait {
        unsafe {
            let mut internal = self.internal.lock();

            // SAFETY: This thread reference never leaves the current thread. Since this references the current thread, it must continue to
            //         exist while this thread is still executing, so extending its lifetime like this is safe.
            let mut thread = (*(&*Thread::current() as *const Thread)).lock();

            assert!(matches!(*thread.state(), ThreadState::Running));

            // SAFETY: The only way for the caller to release the thread lock at this point would be to either call ThreadWait::suspend or
            //         drop the returned ThreadWait, which will unconditionally panic. If the returned ThreadWait is leaked, then the thread
            //         is never unlocked and the improper state updates can never be observed. Obviously, this is undesirable but does not
            //         have any implications for safety guarantees.
            internal.enqueue(&mut thread, self.get_ref());
            ThreadWait(MaybeUninit::new(thread))
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
        if let Some(mut thread) = self.internal.lock().dequeue(self) {
            // SAFETY: A waiting -> ready transition (via suspended) is safe since the event the thread was waiting on has now occurred.
            unsafe {
                *thread.state_mut() = ThreadState::Suspended;
                thread.wake();
            };

            true
        } else {
            false
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
        let mut lock = self.internal.lock();

        while let Some(mut thread) = lock.dequeue(self) {
            // SAFETY: A waiting -> ready transition (via suspended) is safe since the event the thread was waiting on has now occurred.
            unsafe {
                *thread.state_mut() = ThreadState::Suspended;
                thread.wake();
            };

            num_woken += 1;
        };

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