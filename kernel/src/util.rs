use alloc::sync::{Arc, Weak};
use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;

#[repr(transparent)]
#[derive(Debug)]
pub struct SharedUnsafeCell<T: ?Sized>(pub UnsafeCell<T>);

impl<T> SharedUnsafeCell<T> {
    pub const fn new(val: T) -> Self {
        SharedUnsafeCell(UnsafeCell::new(val))
    }
}

impl<T: ?Sized> Deref for SharedUnsafeCell<T> {
    type Target = UnsafeCell<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

unsafe impl<T> Sync for SharedUnsafeCell<T> {}
unsafe impl<T> Send for SharedUnsafeCell<T> {}

#[repr(align(4096))]
pub struct PageAligned<T>(T);

impl<T> PageAligned<T> {
    pub const fn new(val: T) -> PageAligned<T> {
        PageAligned(val)
    }
}

impl<T> Deref for PageAligned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for PageAligned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

trait CloneOrPanic {
    fn clone_or_panic(&self) -> Self;
}

impl<T> CloneOrPanic for T {
    default fn clone_or_panic(&self) -> T {
        panic!("Attempt to clone uncloneable type {}", core::any::type_name::<T>());
    }
}

impl<T: Clone> CloneOrPanic for T {
    fn clone_or_panic(&self) -> T {
        self.clone()
    }
}

pub fn clone_or_panic<T>(val: &T) -> T {
    val.clone_or_panic()
}

trait UnitOrPanic {
    fn unit_or_panic() -> Self;
}

impl<T> UnitOrPanic for T {
    default fn unit_or_panic() -> T {
        panic!("Attempt to create unit value of non-unit type {}", core::any::type_name::<T>());
    }
}

impl UnitOrPanic for () {
    fn unit_or_panic() {}
}

pub fn unit_or_panic<T>() -> T {
    UnitOrPanic::unit_or_panic()
}

#[derive(Debug, Clone)]
pub struct PinWeak<T: ?Sized>(Weak<T>);

impl<T: ?Sized> PinWeak<T> {
    pub fn downgrade(this: &Pin<Arc<T>>) -> PinWeak<T> {
        unsafe { PinWeak(Arc::downgrade(&Pin::into_inner_unchecked(this.clone()))) }
    }

    pub fn upgrade(&self) -> Option<Pin<Arc<T>>> {
        unsafe { self.0.upgrade().map(|arc| Pin::new_unchecked(arc)) }
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

#[derive(Debug)]
pub struct SendPtr<T: ?Sized>(*const T);
unsafe impl<T: ?Sized> Send for SendPtr<T> {}
impl<T: ?Sized> Copy for SendPtr<T> {}
impl<T: ?Sized> Clone for SendPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> SendPtr<T> {
    pub fn new(ptr: *const T) -> Self {
        SendPtr(ptr)
    }

    pub fn unwrap(self) -> *const T {
        self.0
    }
}

pub struct DisplayAsDebug<T: fmt::Display>(T);

impl<T: fmt::Display> DisplayAsDebug<T> {
    pub fn new(val: T) -> DisplayAsDebug<T> {
        DisplayAsDebug(val)
    }
}

impl<T: fmt::Display> fmt::Debug for DisplayAsDebug<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
