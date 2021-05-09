//! Asynchronously resolved values.

use alloc::boxed::Box;
use core::marker::PhantomData;
use core::mem;
use core::pin::Pin;
use core::ptr;

use x86_64::instructions::interrupts;

use crate::sched::wait::ThreadWaitList;
use crate::sync::uninterruptible::{UninterruptibleSpinlock, UninterruptibleSpinlockGuard};

struct FutureWait<T> {
    refs: usize,
    val: Option<T>,
    wait: ThreadWaitList
}

#[derive(Debug)]
enum FutureInternal<T> {
    Waiting(
        *const UninterruptibleSpinlock<FutureWait<T>>,
        PhantomData<UninterruptibleSpinlock<FutureWait<T>>>
    ),
    Done(T)
}

/// Represents a value that will be available when an asynchronous operation completes.
///
/// A future represents an operation that will result in a value that will be available at some indeterminate point in the future. This is
/// often used to represent values that are obtained from hardware I/O and so will be resolved when the hardware completes the request.
///
/// A thread wanting to perform an operation on certain kinds of hardware needs to wait until the hardware signals completion of the
/// operation, usually indicated by an interrupt being raised. During this time, the thread should be put to sleep so that the CPU can run
/// other threads. A future provides a good way of representing this, allowing a thread to block waiting for a value that can be provided
/// in an interrupt handler.
///
/// Note that the model of how futures work here requires that all futures resolve to a value at some point in the future. Creating a future
/// but failing to ever resolve it will leak internal memory used to track futures that are waiting to be resolved and can result in threads
/// being left in a state where they are stuck waiting and cannot be killed normally.
#[derive(Debug)]
pub struct Future<T>(FutureInternal<T>);

impl<T> Future<T> {
    /// Creates a new unresolved [`Future`] that can be fulfilled using the provided [`FutureWriter`].
    #[must_use]
    pub fn new() -> (Future<T>, FutureWriter<T>) {
        let wait = Box::leak(Box::new(UninterruptibleSpinlock::new(FutureWait {
            refs: 1,
            val: None,
            wait: ThreadWaitList::new()
        })));

        (Future(FutureInternal::Waiting(wait, PhantomData)), FutureWriter {
            wait,
            _data: PhantomData
        })
    }

    /// Creates a new [`Future`] that is immediately resolved with the provided value. Calling methods on the returned future will never
    /// block or otherwise fail to return the provided value, even without needing to update the future's readiness.
    pub fn done(val: T) -> Future<T> {
        Future(FutureInternal::Done(val))
    }

    fn do_action<U>(&mut self, f: impl FnOnce(Result<&mut T, UninterruptibleSpinlockGuard<FutureWait<T>>>) -> U) -> U {
        let result = match self.0 {
            FutureInternal::Waiting(ptr, _) => interrupts::without_interrupts(|| unsafe {
                let mut wait_guard = (*ptr).lock();
                let wait = &mut *wait_guard;

                if let Some(ref mut val) = wait.val {
                    wait.refs -= 1;

                    let val = if wait.refs == 0 {
                        let val = wait.val.take().unwrap();

                        mem::drop(wait_guard);
                        Box::from_raw(ptr as *mut UninterruptibleSpinlock<FutureWait<T>>);

                        val
                    } else {
                        crate::util::clone_or_panic(val)
                    };

                    self.0 = FutureInternal::Done(val);
                    Ok(f)
                } else {
                    Err(f(Err(wait_guard)))
                }
            }),
            FutureInternal::Done(_) => Ok(f)
        };

        match result {
            Ok(f) => match self.0 {
                FutureInternal::Waiting(_, _) => unreachable!(),
                FutureInternal::Done(ref mut val) => f(Ok(val))
            },
            Err(wait_result) => wait_result
        }
    }

    /// Blocks the current thread until this future resolves.
    ///
    /// # Panics
    ///
    /// This operation cannot be called from an interrupt handler or while the current thread is in a state in which it cannot block, such
    /// as while holding spinlocks. If this method is called on a future whose value is not immediately available from such a context, it
    /// will panic.
    pub fn block_until_ready(&mut self) {
        loop {
            let done = self.do_action(|state| match state {
                Ok(_) => true,
                Err(wait) => {
                    let suspend = unsafe { Pin::new_unchecked(&wait.wait) }.wait();
                    mem::drop(wait);
                    suspend.suspend();

                    false
                }
            });

            if done {
                break;
            };
        }
    }

    /// Updates this future based on the current state of the request. This operation will never block and so is safe to call from within
    /// an interrupt handler.
    pub fn update_readiness(&mut self) -> bool {
        self.do_action(|state| state.is_ok())
    }

    /// Gets whether this future has been resolved. This operation will never block and so is safe to call from within an interrupt handler.
    ///
    /// Note that this method does not update the state of this future to check if it has been resolved since the last call to
    /// [`Future::update_readiness`]. In general, this method should only be called after calling that method or immediately after receiving
    /// a future to avoid stale results.
    pub fn is_ready(&self) -> bool {
        match self.0 {
            FutureInternal::Waiting(_, _) => false,
            FutureInternal::Done(_) => true
        }
    }

    /// Blocks until this future is resolved, then returns the value it resolved to.
    ///
    /// # Panics
    ///
    /// This operation cannot be called from an interrupt handler or while the current thread is in a state in which it cannot block, such
    /// as while holding spinlocks. If this method is called on a future whose value is not immediately available from such a context, it
    /// will panic. To attempt to unwrap a future without blocking, which can be safely done from an interrupt handler, use
    /// [`Future::try_unwrap`] instead.
    pub fn unwrap_blocking(mut self) -> T {
        self.block_until_ready();

        match mem::replace(&mut self.0, FutureInternal::Waiting(ptr::null(), PhantomData)) {
            FutureInternal::Waiting(_, _) => unreachable!(),
            FutureInternal::Done(val) => {
                mem::forget(self);
                val
            }
        }
    }

    /// Gets the value this future resolved to if calling [`Future::is_ready`] would return true. Otherwise, this method returns an `Err`
    /// variant containing this future so that further handling can be attempted. This operation will never block and so is is safe to call
    /// from within an interrupt handler.
    ///
    /// Note that this method does not update the state of this future to check if it has been resolved since the last call to
    /// [`Future::update_readiness`] and so may return stale results. In general. this method should only be called on a future which has
    /// just been received as part of a non-blocking fast path. If the readiness of this future has potentially not been updated in a while,
    /// [`Future::try_unwrap`] should be used instead.
    pub fn try_unwrap_without_update(mut self) -> Result<T, Future<T>> {
        match self.0 {
            FutureInternal::Waiting(_, _) => Err(self),
            FutureInternal::Done(_) => match mem::replace(&mut self.0, FutureInternal::Waiting(ptr::null(), PhantomData)) {
                FutureInternal::Waiting(_, _) => unreachable!(),
                FutureInternal::Done(val) => {
                    mem::forget(self);
                    Ok(val)
                }
            }
        }
    }

    /// Gets the value this future resolved to if it has been resolved. Otherwise, this method returns an `Err` variant containing this
    /// future so that further handling can be attempted. This operation will never block and so is safe to call from within an interrupt
    /// handler.
    pub fn try_unwrap(mut self) -> Result<T, Future<T>> {
        self.update_readiness();
        self.try_unwrap_without_update()
    }
}

impl<T: Clone> Clone for Future<T> {
    fn clone(&self) -> Future<T> {
        match self.0 {
            FutureInternal::Waiting(ptr, _) => unsafe {
                (*ptr).lock().refs += 1;
                Future(FutureInternal::Waiting(ptr, PhantomData))
            },
            FutureInternal::Done(ref val) => Future(FutureInternal::Done(val.clone()))
        }
    }
}

impl<T> Drop for Future<T> {
    fn drop(&mut self) {
        match self.0 {
            FutureInternal::Waiting(ptr, _) if !ptr.is_null() => unsafe {
                let mut wait = (*ptr).lock();

                wait.refs -= 1;
                if wait.refs == 0 {
                    mem::drop(wait);
                    Box::from_raw(ptr as *mut UninterruptibleSpinlock<FutureWait<T>>);
                };
            },
            _ => {}
        };
    }
}

/// Represents ownership of the "resolution side" of a future. Holding a value of this type allows the caller to resolve its associated
/// future.
///
/// Dropping or leaking a value of this type is generally not advisable, as doing so will cause all threads waiting on this future to hang
/// forever and will leak memory used internally to track the state of unresolved futures. For that reason, attempting to drop a value of
/// this type except by calling [`FutureWriter::finish`].
#[derive(Debug)]
pub struct FutureWriter<T> {
    wait: *const UninterruptibleSpinlock<FutureWait<T>>,
    _data: PhantomData<UninterruptibleSpinlock<FutureWait<T>>>
}

impl<T> FutureWriter<T> {
    /// Resolves the future associated with this writer with the provided value.
    pub fn finish(self, val: T) {
        unsafe {
            let mut wait = (*self.wait).lock();

            wait.refs -= 1;
            if wait.refs != 0 {
                wait.val = Some(val);
                wait.wait.wake_all();
            } else {
                mem::drop(wait);
                Box::from_raw(self.wait as *mut UninterruptibleSpinlock<FutureWait<T>>);
            };

            mem::forget(self);
        };
    }
}

impl<T> Drop for FutureWriter<T> {
    fn drop(&mut self) {
        panic!(
            "FutureWriter for {:?} dropped without having a value given (this causes readers to hang forever)",
            self.wait
        );
    }
}
