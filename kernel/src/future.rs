use core::marker::PhantomData;

use x86_64::instructions::interrupts;

struct FutureWait<T> {
    refs: usize,
    val: Option<T>
}

#[derive(Debug)]
enum FutureInternal<T> {
    Waiting(*const spin::Mutex<FutureWait<T>>, PhantomData<spin::Mutex<FutureWait<T>>>),
    Done(T)
}

#[derive(Debug)]
pub struct Future<T>(FutureInternal<T>);

impl <T> Future<T> {
    pub fn new() -> (Future<T>, FutureWriter<T>) {
        // TODO Allocate IoFutureWait
        let wait = core::ptr::null();

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

    fn do_action<U>(&mut self, f: impl FnOnce (Result<&mut T, spin::MutexGuard<FutureWait<T>>>) -> U) -> U {
        let result = match self.0 {
            FutureInternal::Waiting(ptr, _) => interrupts::without_interrupts(|| unsafe {
                let mut wait_guard = (*ptr).lock();
                let wait = &mut *wait_guard;

                if let Some(ref mut val) = wait.val {
                    wait.refs -= 1;

                    let val = if wait.refs == 0 {
                        let val = wait.val.take().unwrap();

                        spin::MutexGuard::leak(wait_guard);
                        core::ptr::drop_in_place(ptr as *mut spin::Mutex<FutureWait<T>>);
                        // TODO Free IoFutureWait

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
                    // TODO Enqueue current thread on the wait queue
                    core::mem::drop(wait);
                    // TODO Put the current thread to sleep
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

        match core::mem::replace(&mut self.0, FutureInternal::Waiting(core::ptr::null(), PhantomData)) {
            FutureInternal::Waiting(_, _) => unreachable!(),
            FutureInternal::Done(val) => {
                core::mem::forget(self);
                val
            }
        }
    }

    pub fn try_unwrap_without_update(mut self) -> Result<T, Future<T>> {
        match self.0 {
            FutureInternal::Waiting(_, _) => Err(self),
            FutureInternal::Done(_) => match core::mem::replace(&mut self.0, FutureInternal::Waiting(core::ptr::null(), PhantomData)) {
                FutureInternal::Waiting(_, _) => unreachable!(),
                FutureInternal::Done(val) => {
                    core::mem::forget(self);
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
                    spin::MutexGuard::leak(wait);
                    core::ptr::drop_in_place(ptr as *mut spin::Mutex<FutureWait<T>>);
                    // TODO Free IoFutureWait
                };
            },
            _ => {}
        };
    }
}

#[derive(Debug)]
pub struct FutureWriter<T> {
    wait: *const spin::Mutex<FutureWait<T>>,
    _data: PhantomData<spin::Mutex<FutureWait<T>>>
}

impl <T> FutureWriter<T> {
    pub fn finish(self, val: T) {
        unsafe {
            let mut wait = (*self.wait).lock();

            wait.refs -= 1;
            if wait.refs != 0 {
                wait.val = Some(val);
                // TODO Wake up threads waiting on this future
            } else {
                spin::MutexGuard::leak(wait);
                core::ptr::drop_in_place(self.wait as *mut spin::Mutex<FutureWait<T>>);
                // TODO Free IoFutureWait
            };

            core::mem::forget(self);
        };
    }
}

impl <T> Drop for FutureWriter<T> {
    fn drop(&mut self) {
        panic!("IoFutureWriter for {:?} dropped without having a value given (this causes readers to hang forever)", self.wait);
    }
}
