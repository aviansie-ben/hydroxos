//! Asynchronously resolved values.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem;
use core::mem::MaybeUninit;
use core::ptr;

use crate::sched;
use crate::sched::task::Thread;
use crate::sched::wait::ThreadWaitList;
use crate::sync::uninterruptible::{UninterruptibleSpinlock, UninterruptibleSpinlockGuard};
use crate::util::SendPtr;

type FutureWaitAction = dyn FnOnce(*const (), &mut FutureWaitGenericLock) + Send;

struct FutureWaitGenericState {
    wait_refs: usize,
    val_refs: usize,
    resolved: bool,
    actions: Vec<Box<FutureWaitAction>>
}

struct FutureWaitGeneric {
    state: UninterruptibleSpinlock<FutureWaitGenericState>,
    wait: ThreadWaitList
}

impl FutureWaitGeneric {
    pub fn lock(&self) -> FutureWaitGenericLock {
        FutureWaitGenericLock {
            state: self.state.lock(),
            wait: &self.wait
        }
    }
}

struct FutureWaitGenericLock<'a> {
    state: UninterruptibleSpinlockGuard<'a, FutureWaitGenericState>,
    wait: &'a ThreadWaitList
}

impl<'a> FutureWaitGenericLock<'a> {
    pub fn wait(self) {
        let FutureWaitGenericLock { state, wait } = self;

        assert!(!state.resolved);

        let wait = wait.wait();
        drop(state);
        wait.suspend();
    }
}

#[repr(C)]
pub struct FutureWait<T> {
    generic: FutureWaitGeneric,
    val: UnsafeCell<MaybeUninit<T>>
}

impl<T> FutureWait<T> {
    fn new(wait_refs: usize, val_refs: usize) -> *const FutureWait<T> {
        Box::into_raw(Box::new(FutureWait {
            generic: FutureWaitGeneric {
                state: UninterruptibleSpinlock::new(FutureWaitGenericState {
                    wait_refs,
                    val_refs,
                    resolved: false,
                    actions: vec![]
                }),
                wait: ThreadWaitList::new()
            },
            val: UnsafeCell::new(MaybeUninit::uninit())
        }))
    }

    unsafe fn destroy(ptr: *const FutureWait<T>) {
        drop(Box::from_raw(ptr as *mut FutureWait<T>));
    }

    unsafe fn dec_val_ref(&self, lock: &mut FutureWaitGenericLock) {
        assert!(self.generic.state.is_guarded_by(&lock.state));
        assert!(lock.state.val_refs > 0);

        lock.state.val_refs -= 1;
        if lock.state.resolved && lock.state.val_refs == 0 {
            ptr::read((*self.val.get()).as_ptr());
        }
    }

    unsafe fn take_val(&self, lock: &mut FutureWaitGenericLock) -> T {
        assert!(self.generic.state.is_guarded_by(&lock.state));
        assert!(lock.state.val_refs > 0);
        assert!(lock.state.resolved);

        lock.state.val_refs -= 1;
        if lock.state.val_refs == 0 {
            ptr::read((*self.val.get()).as_ptr())
        } else {
            crate::util::clone_or_panic(&*(*self.val.get()).as_ptr())
        }
    }
}

#[derive(Debug)]
enum FutureInternalUnresolved<T> {
    WithVal(*const FutureWait<T>),
    WithoutVal(*const FutureWaitGeneric, fn(*const FutureWaitGeneric))
}

unsafe impl<T: Send> Send for FutureInternalUnresolved<T> {}
unsafe impl<T: Send> Sync for FutureInternalUnresolved<T> {}

impl<T> FutureInternalUnresolved<T> {
    unsafe fn dec_wait_ref(&mut self, wait: FutureWaitGenericLock) {
        match *self {
            FutureInternalUnresolved::WithVal(ptr) => {
                Future::dec_wait_ref(ptr, wait);
            },
            FutureInternalUnresolved::WithoutVal(ptr, free) => {
                Future::dec_wait_ref_generic(ptr, free, wait);
            }
        }
    }

    unsafe fn try_resolve(mut self) -> Result<T, (FutureInternalUnresolved<T>, FutureWaitGenericLock<'static>)> {
        let mut lock = match self {
            FutureInternalUnresolved::WithVal(ptr) => unsafe { (*ptr).generic.lock() },
            FutureInternalUnresolved::WithoutVal(ptr, _) => unsafe { (*ptr).lock() }
        };

        if lock.state.resolved {
            let val = match self {
                FutureInternalUnresolved::WithVal(ptr) => unsafe { (*ptr).take_val(&mut lock) },
                FutureInternalUnresolved::WithoutVal(_, _) => crate::util::unit_or_panic()
            };

            self.dec_wait_ref(lock);
            mem::forget(self);
            Ok(val)
        } else {
            Err((self, lock))
        }
    }
}

impl<T> Drop for FutureInternalUnresolved<T> {
    fn drop(&mut self) {
        let mut lock = match *self {
            FutureInternalUnresolved::WithVal(ptr) => unsafe { (*ptr).generic.lock() },
            FutureInternalUnresolved::WithoutVal(ptr, _) => unsafe { (*ptr).lock() }
        };

        if let FutureInternalUnresolved::WithVal(ptr) = *self {
            unsafe {
                (*ptr).dec_val_ref(&mut lock);
            }
        }

        unsafe {
            self.dec_wait_ref(lock);
        }
    }
}

#[derive(Debug)]
enum FutureInternal<T> {
    Unresolved(FutureInternalUnresolved<T>),
    Done(T),
    Invalid
}

impl<T> FutureInternal<T> {
    fn update_state(&mut self) -> Option<FutureWaitGenericLock> {
        match mem::replace(self, FutureInternal::Invalid) {
            FutureInternal::Unresolved(unresolved) => match unsafe { unresolved.try_resolve() } {
                Ok(val) => {
                    *self = FutureInternal::Done(val);
                    None
                },
                Err((unresolved, lock)) => {
                    *self = FutureInternal::Unresolved(unresolved);
                    Some(lock)
                }
            },
            FutureInternal::Done(val) => {
                *self = FutureInternal::Done(val);
                None
            },
            FutureInternal::Invalid => {
                panic!("future is in invalid state");
            }
        }
    }
}

impl<T: Send + Sync + Clone> Clone for FutureInternal<T> {
    fn clone(&self) -> Self {
        match *self {
            FutureInternal::Unresolved(FutureInternalUnresolved::WithVal(ptr)) => unsafe {
                let mut lock = (*ptr).generic.lock();

                if lock.state.resolved {
                    FutureInternal::Done(crate::util::clone_or_panic((*(*ptr).val.get()).assume_init_ref()))
                } else {
                    lock.state.wait_refs += 1;
                    lock.state.val_refs += 1;
                    FutureInternal::Unresolved(FutureInternalUnresolved::WithVal(ptr))
                }
            },
            FutureInternal::Unresolved(FutureInternalUnresolved::WithoutVal(ptr, free)) => unsafe {
                let mut lock = (*ptr).lock();

                if lock.state.resolved {
                    FutureInternal::Done(crate::util::unit_or_panic())
                } else {
                    lock.state.wait_refs += 1;
                    FutureInternal::Unresolved(FutureInternalUnresolved::WithoutVal(ptr, free))
                }
            },
            FutureInternal::Done(ref val) => FutureInternal::Done(val.clone()),
            FutureInternal::Invalid => FutureInternal::Invalid
        }
    }
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
#[must_use]
pub struct Future<T>(FutureInternal<T>);

impl<T> Future<T> {
    /// Creates a new unresolved [`Future`] that can be fulfilled using the provided [`FutureWriter`].
    ///
    /// If there's a need to create a [`FutureWriter`] without immediately requiring an associated [`Future`], it's generally preferred to
    /// call [`FutureWriter::new`].
    pub fn new() -> (Future<T>, FutureWriter<T>) {
        let wait = FutureWait::new(2, 1);

        (
            Future(FutureInternal::Unresolved(FutureInternalUnresolved::WithVal(wait))),
            FutureWriter { wait, _data: PhantomData }
        )
    }

    /// Creates a new [`Future`] that is immediately resolved with the provided value. Calling methods on the returned future will never
    /// block or otherwise fail to return the provided value, even without needing to update the future's readiness.
    pub const fn done(val: T) -> Future<T> {
        Future(FutureInternal::Done(val))
    }

    fn do_action<U>(&mut self, f: impl FnOnce(Result<&mut T, FutureWaitGenericLock>) -> U) -> U {
        match self.0.update_state() {
            Some(lock) => f(Err(lock)),
            s @ None => {
                drop(s);
                f(Ok(match self.0 {
                    FutureInternal::Done(ref mut val) => val,
                    _ => unreachable!()
                }))
            }
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
                    wait.wait();
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
            FutureInternal::Unresolved(_) => false,
            FutureInternal::Done(_) => true,
            FutureInternal::Invalid => unreachable!()
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

        match mem::replace(&mut self.0, FutureInternal::Invalid) {
            FutureInternal::Done(val) => {
                mem::forget(self);
                val
            },
            _ => unreachable!()
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
    pub fn try_unwrap_without_update(self) -> Result<T, Future<T>> {
        match self.0 {
            FutureInternal::Unresolved(unresolved) => Err(Future(FutureInternal::Unresolved(unresolved))),
            FutureInternal::Done(val) => Ok(val),
            FutureInternal::Invalid => unreachable!()
        }
    }

    /// Gets the value this future resolved to if it has been resolved. Otherwise, this method returns an `Err` variant containing this
    /// future so that further handling can be attempted. This operation will never block and so is safe to call from within an interrupt
    /// handler.
    pub fn try_unwrap(mut self) -> Result<T, Future<T>> {
        self.update_readiness();
        self.try_unwrap_without_update()
    }

    /// Runs the provided callback when this [`Future`] is resolved. This callback will be run immediately if the future is already
    /// resolved. Otherwise, it will be run when [`FutureWriter::finish`] is called, from the context of the code that calls that method.
    /// As a result, the provided callback may be run in a different thread than the current thread or even in the context of an interrupt
    /// handler.
    ///
    /// # Panics
    ///
    /// A panic will occur when calling the provided callback if it attempts to perform a blocking operation.
    pub fn when_resolved(self, f: impl FnOnce(T) + Send + 'static) {
        match self.0 {
            FutureInternal::Unresolved(unresolved) => {
                let has_val = matches!(unresolved, FutureInternalUnresolved::WithVal(_));
                match unsafe { unresolved.try_resolve() } {
                    Ok(val) => f(val),
                    Err((unresolved, mut lock)) => {
                        lock.state.actions.push(Box::new(move |ptr, lock| {
                            let val = if has_val {
                                unsafe { (*(ptr as *const FutureWait<T>)).take_val(lock) }
                            } else {
                                crate::util::unit_or_panic()
                            };

                            f(val);
                        }));

                        lock.state.wait_refs -= 1;
                        mem::forget(unresolved);
                    }
                }
            },
            FutureInternal::Done(val) => {
                f(val);
            },
            FutureInternal::Invalid => unreachable!()
        }
    }

    /// Creates a future that resolves to `()` when this future is resolved. This allows for multiple futures to be created that will
    /// resolve along with another future, even if the value in that future does not implement [`Clone`].
    pub fn without_val(&self) -> Future<()> {
        match self.0 {
            FutureInternal::Unresolved(FutureInternalUnresolved::WithVal(ptr)) => unsafe {
                let mut lock = (*ptr).generic.lock();

                if lock.state.resolved {
                    Future::done(())
                } else {
                    lock.state.wait_refs += 1;
                    let generic_ptr = &(*ptr).generic as *const _;
                    assert_eq!(generic_ptr as *const (), ptr as *const ());
                    Future(FutureInternal::Unresolved(FutureInternalUnresolved::WithoutVal(
                        generic_ptr,
                        |ptr| FutureWait::destroy(ptr as *const FutureWait<T>)
                    )))
                }
            },
            FutureInternal::Unresolved(FutureInternalUnresolved::WithoutVal(ptr, free)) => unsafe {
                (*ptr).lock().state.wait_refs += 1;
                Future(FutureInternal::Unresolved(FutureInternalUnresolved::WithoutVal(ptr, free)))
            },
            FutureInternal::Done(_) => Future::done(()),
            FutureInternal::Invalid => unreachable!()
        }
    }

    unsafe fn dec_wait_ref(ptr: *const FutureWait<T>, mut wait: FutureWaitGenericLock) {
        wait.state.wait_refs -= 1;
        if wait.state.wait_refs != 0 {
            wait.wait.wake_all();
        } else {
            drop(wait);
            FutureWait::destroy(ptr);
        }
    }
}

impl<T: Send + 'static> Future<T> {
    /// Runs the provided callback in a soft interrupt after this [`Future`] is resolved.
    ///
    /// # Panics
    ///
    /// A panic will occur when calling the provided callback if it attempts to perform a blocking operation.
    pub fn when_resolved_soft(self, f: impl FnOnce(T) + Send + 'static) {
        self.when_resolved(move |val| {
            sched::enqueue_soft_interrupt(move || {
                f(val);
            });
        });
    }
}

impl Future<()> {
    unsafe fn dec_wait_ref_generic(ptr: *const FutureWaitGeneric, free: fn(*const FutureWaitGeneric), mut wait: FutureWaitGenericLock) {
        wait.state.wait_refs -= 1;
        if wait.state.wait_refs != 0 {
            wait.wait.wake_all();
        } else {
            drop(wait);
            (free)(ptr);
        }
    }

    /// Creates a future that resolves once all of the futures in the provided iterator have resolved.
    pub fn all(fs: impl IntoIterator<Item = Future<()>>) -> Future<()> {
        let (future, writer) = Future::new();
        let wait = SendPtr::new(writer.into_raw());

        unsafe {
            (*(*wait.unwrap()).val.get()) = MaybeUninit::new(usize::MAX);
        }

        let mut num_futures = 0;
        for f in fs {
            num_futures += 1;

            if num_futures == usize::MAX {
                panic!("Iterator passed to Future::all is too long");
            }

            f.when_resolved(move |_| unsafe {
                let wait_generic = (*wait.unwrap()).generic.lock();

                if *(*(*wait.unwrap()).val.get()).as_ptr() == 1 {
                    FutureWriter::finish_internal(wait.unwrap(), wait_generic);
                } else {
                    *(*(*wait.unwrap()).val.get()).as_mut_ptr() -= 1;
                };
            });
        }

        unsafe {
            let wait_generic = (*wait.unwrap()).generic.lock();

            *(*(*wait.unwrap()).val.get()).as_mut_ptr() -= usize::MAX - num_futures;

            if *(*(*wait.unwrap()).val.get()).as_ptr() == 0 {
                FutureWriter::finish_internal(wait.unwrap(), wait_generic);
            }
        }

        future.without_val()
    }

    /// Creates a future that resolves once one of the futures in the provided iterator has resolved. The value of the returned future once
    /// resolved is the index of the first future within the provided iterator that resolved.
    ///
    /// This function will return `Err(())` if provided with an empty iterator. Creating a future in this case would result in a future
    /// which could never resolve, which is generally not desired behaviour and could lead to threads hanging in an uninterruptible state.
    pub fn any(fs: impl IntoIterator<Item = Future<()>>) -> Result<Future<usize>, ()> {
        let (future, writer) = Future::new();
        let wait = SendPtr::new(writer.into_raw());

        let mut was_empty = true;
        for (i, f) in fs.into_iter().enumerate() {
            was_empty = false;

            unsafe {
                (*wait.unwrap()).generic.lock().state.wait_refs += 1;
            }

            f.when_resolved(move |_| unsafe {
                let wait_generic = (*wait.unwrap()).generic.lock();

                if !wait_generic.state.resolved {
                    *(*wait.unwrap()).val.get() = MaybeUninit::new(i);
                    FutureWriter::finish_internal(wait.unwrap(), wait_generic);
                } else {
                    Future::dec_wait_ref(wait.unwrap(), wait_generic);
                }
            })
        }

        unsafe {
            (*wait.unwrap()).generic.lock().state.wait_refs -= 1;
        }

        if !was_empty {
            Ok(future)
        } else {
            Err(())
        }
    }
}

impl<T: Send + Sync + Clone> Clone for Future<T> {
    fn clone(&self) -> Self {
        Future(self.0.clone())
    }
}

/// Represents ownership of the "resolution side" of a future. Holding a value of this type allows the caller to resolve its associated
/// future.
///
/// Dropping or leaking a value of this type is generally not advisable, as doing so will cause all threads waiting on this future to hang
/// forever and will leak memory used internally to track the state of unresolved futures. For that reason, attempting to drop a value of
/// this type except by calling [`FutureWriter::finish`] will panic.
#[derive(Debug)]
#[must_use]
pub struct FutureWriter<T> {
    wait: *const FutureWait<T>,
    _data: PhantomData<FutureWait<T>>
}

impl<T> FutureWriter<T> {
    unsafe fn finish_internal(ptr: *const FutureWait<T>, mut wait: FutureWaitGenericLock) {
        wait.state.resolved = true;

        let actions = mem::take(&mut wait.state.actions);
        if !actions.is_empty() {
            Thread::run_non_blocking(|| {
                for a in actions.into_iter() {
                    a(ptr as *const (), &mut wait);
                }
            });
        }

        Future::dec_wait_ref(ptr, wait);
    }

    /// Creates a new writer without yet creating an associated [`Future`].
    ///
    /// This is useful for creating a [`Future`] for a known future event and stashing its writer somewhere before it is known whether any
    /// consumers will actually exist. In order for the resolved value to be read, [`FutureWriter::as_future`] will need to be called to
    /// create a [`Future`] for the returned writer.
    ///
    /// If a [`Future`] that would resolve from this writer is desired immediately, it's preferred to call [`Future::new`] instead.
    pub fn new() -> FutureWriter<T> {
        FutureWriter {
            wait: FutureWait::new(1, 0),
            _data: PhantomData
        }
    }

    /// Creates a new [`Future`] that will resolve to the value written to this writer.
    pub fn as_future(&self) -> Future<T> {
        let mut guard = unsafe { (*self.wait).generic.lock() };

        guard.state.wait_refs += 1;
        guard.state.val_refs += 1;

        Future(FutureInternal::Unresolved(FutureInternalUnresolved::WithVal(self.wait)))
    }

    /// Resolves the future associated with this writer with the provided value.
    pub fn finish(self, val: T) {
        unsafe {
            let wait = (*self.wait).generic.lock();

            if wait.state.val_refs != 0 {
                *(*self.wait).val.get() = MaybeUninit::new(val);
            }
            FutureWriter::finish_internal(self.wait, wait);
            mem::forget(self);
        };
    }

    pub fn into_raw(self) -> *const FutureWait<T> {
        let wait = self.wait;
        mem::forget(self);
        wait
    }

    pub unsafe fn from_raw(ptr: *const FutureWait<T>) -> FutureWriter<T> {
        FutureWriter {
            wait: ptr,
            _data: PhantomData
        }
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

unsafe impl<T: Send> Send for FutureWriter<T> {}
unsafe impl<T: Send> Sync for FutureWriter<T> {}

#[cfg(test)]
mod test {
    use core::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    static ACTION_RUN_FLAG: AtomicBool = AtomicBool::new(false);

    #[test_case]
    fn test_done() {
        assert!(Future::done(0xdead).is_ready());
        assert_eq!(0xdead, Future::done(0xdead).unwrap_blocking());
        assert_eq!(Some(0xdead), Future::done(0xdead).try_unwrap_without_update().ok());
        assert_eq!(Some(0xdead), Future::done(0xdead).try_unwrap().ok());

        ACTION_RUN_FLAG.store(false, Ordering::Relaxed);
        Future::done(0xdead).when_resolved(|_| {
            ACTION_RUN_FLAG.store(true, Ordering::Relaxed);
        });
        assert!(ACTION_RUN_FLAG.load(Ordering::Relaxed));
    }

    #[test_case]
    fn test_is_ready() {
        let (mut future, writer) = Future::new();

        assert!(!future.is_ready());

        writer.finish(0xdead);
        assert!(!future.is_ready());

        future.update_readiness();
        assert!(future.is_ready());
    }

    #[test_case]
    fn test_try_unwrap() {
        let (future, writer) = Future::new();
        let future = match future.try_unwrap() {
            Err(future) => future,
            Ok(val) => {
                panic!("Future was resolved with {:?} early", val);
            }
        };

        writer.finish(0xdead);
        assert_eq!(Some(0xdead), future.try_unwrap().ok());
    }

    #[test_case]
    fn test_try_unwrap_without_update() {
        let (future, writer) = Future::new();
        let future = match future.try_unwrap_without_update() {
            Err(future) => future,
            Ok(val) => {
                panic!("Future was resolved with {:?} early", val);
            }
        };

        writer.finish(0xdead);
        let mut future = match future.try_unwrap_without_update() {
            Err(future) => future,
            Ok(val) => {
                panic!("Future was resolved with {:?} early", val);
            }
        };

        future.update_readiness();
        assert_eq!(Some(0xdead), future.try_unwrap_without_update().ok());
    }

    #[test_case]
    fn test_when_resolved() {
        let (future, writer) = Future::new();

        ACTION_RUN_FLAG.store(false, Ordering::Relaxed);
        future.when_resolved(|_| {
            ACTION_RUN_FLAG.store(true, Ordering::Relaxed);
        });

        assert!(!ACTION_RUN_FLAG.load(Ordering::Relaxed));
        writer.finish(0xdead);
        assert!(ACTION_RUN_FLAG.load(Ordering::Relaxed));
    }

    #[test_case]
    fn test_without_val() {
        let (mut future, writer) = {
            let (future, writer) = Future::new();
            (future.without_val(), writer)
        };

        assert!(!future.is_ready());

        writer.finish(0xdead);
        future.update_readiness();
        assert!(future.is_ready());
        assert!(future.try_unwrap_without_update().is_ok());
    }

    #[test_case]
    fn test_without_val_after_resolve() {
        let (future, future_with_val) = {
            let (future, writer) = Future::new();
            writer.finish(0xdead);

            (future.without_val(), future)
        };

        assert!(!future_with_val.is_ready());
        assert!(future.is_ready());
        assert!(future.try_unwrap_without_update().is_ok());
    }

    #[test_case]
    fn test_without_val_when_resolved() {
        let (future, writer) = {
            let (future, writer) = Future::new();

            (future.without_val(), writer)
        };

        ACTION_RUN_FLAG.store(false, Ordering::Relaxed);
        future.when_resolved(|_| {
            ACTION_RUN_FLAG.store(true, Ordering::Relaxed);
        });

        assert!(!ACTION_RUN_FLAG.load(Ordering::Relaxed));
        writer.finish(0xdead);
        assert!(ACTION_RUN_FLAG.load(Ordering::Relaxed));
    }

    #[test_case]
    fn test_clone() {
        let (future1, writer) = Future::new();
        let mut future2 = future1.clone();

        writer.finish(0xdead);

        future2.update_readiness();
        assert!(!future1.is_ready());
        assert!(future2.is_ready());

        assert_eq!(Some(0xdead), future1.try_unwrap().ok());
        assert_eq!(Some(0xdead), future2.try_unwrap().ok());
    }

    #[test_case]
    fn test_all() {
        let (future1, writer1) = Future::new();
        let (future2, writer2) = Future::new();

        let mut all = Future::all([future1, future2]);

        all.update_readiness();
        assert!(!all.is_ready());

        writer1.finish(());
        all.update_readiness();
        assert!(!all.is_ready());

        writer2.finish(());
        all.update_readiness();
        assert!(all.is_ready());
    }

    #[test_case]
    fn test_all_already_resolved() {
        let (future1, writer1) = Future::new();
        let (future2, writer2) = Future::new();

        writer1.finish(());
        writer2.finish(());

        assert!(Future::all([future1, future2]).is_ready());
    }

    #[test_case]
    fn test_all_empty() {
        assert!(Future::all([]).is_ready());
    }

    #[test_case]
    fn test_any() {
        let (future1, writer1) = Future::new();
        let (future2, writer2) = Future::new();

        let mut any = Future::any([future1, future2]).unwrap();

        any.update_readiness();
        assert!(!any.is_ready());

        writer2.finish(());
        assert_eq!(Some(1), any.try_unwrap().ok());

        writer1.finish(());
    }

    #[test_case]
    fn test_any_already_resolved() {
        let (future1, writer1) = Future::new();
        let (future2, writer2) = Future::new();

        writer1.finish(());
        writer2.finish(());

        assert_eq!(Some(0), Future::any([future1, future2]).unwrap().try_unwrap().ok());
    }

    #[test_case]
    fn test_any_empty() {
        assert!(Future::any([]).is_err());
    }
}
