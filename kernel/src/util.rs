use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
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

    pub fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    pub fn with_lock<U>(&self, f: impl FnOnce (&mut T) -> U) -> U {
        interrupts::without_interrupts(|| {
            let mut lock = self.0.lock();

            f(lock.deref_mut())
        })
    }

    pub fn try_with_lock<U>(&self, f: impl FnOnce(Option<&mut T>) -> U) -> U {
        interrupts::without_interrupts(|| {
            let mut lock = self.0.try_lock();

            f(lock.as_mut().map(|lock| lock.deref_mut()))
        })
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
