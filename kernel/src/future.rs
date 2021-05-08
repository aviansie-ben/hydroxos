use core::marker::PhantomData;
use core::mem;
use core::ptr;
use alloc::boxed::Box;

use x86_64::instructions::interrupts;

use crate::sched::wait::ThreadWaitList;
use crate::util::{InterruptDisableSpinlock, InterruptDisableSpinlockGuard};
use core::pin::Pin;

struct FutureWait<T> {
    refs: usize,
    val: Option<T>,
    wait: ThreadWaitList
}

#[derive(Debug)]
enum FutureInternal<T> {
    Waiting(*const InterruptDisableSpinlock<FutureWait<T>>, PhantomData<InterruptDisableSpinlock<FutureWait<T>>>),
    Done(T)
}

#[derive(Debug)]
pub struct Future<T>(FutureInternal<T>);

impl <T> Future<T> {
    pub fn new() -> (Future<T>, FutureWriter<T>) {
        let wait = Box::leak(Box::new(InterruptDisableSpinlock::new(FutureWait {
            refs: 1,
            val: None,
            wait: ThreadWaitList::new()
        })));

        (
            Future(FutureInternal::Waiting(wait, PhantomData)),
            FutureWriter {
                wait,
                _data: PhantomData
            }
        )
    }

    pub fn done(val: T) -> Future<T> {
        Future(FutureInternal::Done(val))
    }

    fn do_action<U>(&mut self, f: impl FnOnce (Result<&mut T, InterruptDisableSpinlockGuard<FutureWait<T>>>) -> U) -> U {
        let result = match self.0 {
            FutureInternal::Waiting(ptr, _) => interrupts::without_interrupts(|| unsafe {
                let mut wait_guard = (*ptr).lock();
                let wait = &mut *wait_guard;

                if let Some(ref mut val) = wait.val {
                    wait.refs -= 1;

                    let val = if wait.refs == 0 {
                        let val = wait.val.take().unwrap();

                        mem::drop(wait_guard);
                        Box::from_raw(ptr as *mut InterruptDisableSpinlock<FutureWait<T>>);

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
        };
    }

    pub fn update_readiness(&mut self) -> bool {
        self.do_action(|state| state.is_ok())
    }

    pub fn is_ready(&self) -> bool {
        match self.0 {
            FutureInternal::Waiting(_, _) => false,
            FutureInternal::Done(_) => true
        }
    }

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

    pub fn try_unwrap(mut self) -> Result<T, Future<T>> {
        self.update_readiness();
        self.try_unwrap_without_update()
    }
}

impl <T: Clone> Clone for Future<T> {
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

impl <T> Drop for Future<T> {
    fn drop(&mut self) {
        match self.0 {
            FutureInternal::Waiting(ptr, _) if !ptr.is_null() => unsafe {
                let mut wait = (*ptr).lock();

                wait.refs -= 1;
                if wait.refs == 0 {
                    mem::drop(wait);
                    Box::from_raw(ptr as *mut InterruptDisableSpinlock<FutureWait<T>>);
                };
            },
            _ => {}
        };
    }
}

#[derive(Debug)]
pub struct FutureWriter<T> {
    wait: *const InterruptDisableSpinlock<FutureWait<T>>,
    _data: PhantomData<InterruptDisableSpinlock<FutureWait<T>>>
}

impl <T> FutureWriter<T> {
    pub fn finish(self, val: T) {
        unsafe {
            let mut wait = (*self.wait).lock();

            wait.refs -= 1;
            if wait.refs != 0 {
                wait.val = Some(val);
                wait.wait.wake_all();
            } else {
                mem::drop(wait);
                Box::from_raw(self.wait as *mut InterruptDisableSpinlock<FutureWait<T>>);
            };

            mem::forget(self);
        };
    }
}

impl <T> Drop for FutureWriter<T> {
    fn drop(&mut self) {
        panic!("IoFutureWriter for {:?} dropped without having a value given (this causes readers to hang forever)", self.wait);
    }
}
