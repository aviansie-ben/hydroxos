use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use alloc::sync::{Arc, Weak};

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
