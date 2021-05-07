use core::cell::{Cell, UnsafeCell};
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use alloc::sync::{Arc, Weak};

use x86_64::instructions::interrupts;

#[repr(transparent)]
pub struct SharedUnsafeCell<T>(pub UnsafeCell<T>);

impl <T> SharedUnsafeCell<T> {
    pub const fn new(val: T) -> Self {
        SharedUnsafeCell(UnsafeCell::new(val))
    }
}

impl <T> Deref for SharedUnsafeCell<T> {
    type Target = UnsafeCell<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

unsafe impl <T> Sync for SharedUnsafeCell<T> {}
unsafe impl <T> Send for SharedUnsafeCell<T> {}

#[repr(align(4096))]
pub struct PageAligned<T>(T);

impl <T> PageAligned<T> {
    pub const fn new(val: T) -> PageAligned<T> {
        PageAligned(val)
    }
}

impl <T> Deref for PageAligned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl <T> DerefMut for PageAligned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug)]
pub struct InterruptDisableSpinlock<T>(spin::Mutex<T>);

impl <T> InterruptDisableSpinlock<T> {
    pub const fn new(val: T) -> InterruptDisableSpinlock<T> {
        InterruptDisableSpinlock(spin::Mutex::new(val))
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.0.get_mut()
    }

    pub fn into_inner(self) -> T {
        self.0.into_inner()
    }

    pub fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    pub fn lock(&self) -> InterruptDisableSpinlockGuard<T> {
        let interrupt_disabler = InterruptDisabler::new();
        let guard = self.0.lock();

        InterruptDisableSpinlockGuard(guard, interrupt_disabler)
    }

    pub fn try_lock(&self) -> Option<InterruptDisableSpinlockGuard<T>> {
        let interrupt_disabler = InterruptDisabler::new();

        if let Some(guard) = self.0.try_lock() {
            Some(InterruptDisableSpinlockGuard(guard, interrupt_disabler))
        } else {
            None
        }
    }

    pub fn with_lock<U>(&self, f: impl FnOnce (&mut T) -> U) -> U {
        let mut lock = self.lock();
        f(lock.deref_mut())
    }

    pub fn try_with_lock<U>(&self, f: impl FnOnce(Option<&mut T>) -> U) -> U {
        if let Some(mut lock) = self.try_lock() {
            f(Some(lock.deref_mut()))
        } else {
            f(None)
        }
    }
}

#[thread_local]
static INTERRUPT_DISABLER_STATE: Cell<(usize, bool)> = Cell::new((0, false));

pub struct InterruptDisabler(());

impl InterruptDisabler {
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

    pub fn num_held() -> usize {
        INTERRUPT_DISABLER_STATE.get().0
    }

    pub(crate) fn force_disable_after_release() {
        INTERRUPT_DISABLER_STATE.set((InterruptDisabler::num_held(), false));
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

pub struct InterruptDisableSpinlockGuard<'a, T>(spin::MutexGuard<'a, T>, InterruptDisabler);

impl <'a, T> Deref for InterruptDisableSpinlockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl <'a, T> DerefMut for InterruptDisableSpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

trait CloneOrPanic {
    fn clone_or_panic(&self) -> Self;
}

impl <T> CloneOrPanic for T {
    default fn clone_or_panic(&self) -> T {
        panic!("Attempt to clone uncloneable type {}", core::any::type_name::<T>());
    }
}

impl <T: Clone> CloneOrPanic for T {
    fn clone_or_panic(&self) -> T {
        self.clone()
    }
}

pub fn clone_or_panic<T>(val: &T) -> T {
    val.clone_or_panic()
}

#[derive(Debug, Clone)]
pub struct PinWeak<T: ?Sized>(Weak<T>);

impl <T: ?Sized> PinWeak<T> {
    pub fn downgrade(this: &Pin<Arc<T>>) -> PinWeak<T> {
        unsafe {
            PinWeak(Arc::downgrade(&Pin::into_inner_unchecked(this.clone())))
        }
    }

    pub fn upgrade(&self) -> Option<Pin<Arc<T>>> {
        unsafe {
            self.0.upgrade().map(|arc| Pin::new_unchecked(arc))
        }
    }

    pub fn as_ptr(&self) -> *const T {
        self.0.as_ptr()
    }

    pub unsafe fn as_weak(&self) -> &Weak<T> {
        &self.0
    }

    pub unsafe fn into_weak(self) -> Weak<T> {
        self.0
    }

    pub unsafe fn from_weak(weak: Weak<T>) -> PinWeak<T> {
        PinWeak(weak)
    }
}
