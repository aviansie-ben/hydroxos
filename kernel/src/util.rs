use alloc::sync::{Arc, Weak};
use core::cell::UnsafeCell;
use core::fmt;
use core::mem::MaybeUninit;
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

pub struct ArrayDeque<T, const N: usize> {
    head: usize,
    len: usize,
    data: [MaybeUninit<T>; N]
}

impl<T, const N: usize> ArrayDeque<T, N> {
    pub fn new() -> Self {
        Self {
            head: 0,
            len: 0,
            data: MaybeUninit::uninit_array()
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn idx(head: usize, idx: usize) -> usize {
        if idx >= N - head {
            idx - (N - head)
        } else {
            head + idx
        }
    }

    fn tail_exclusive(&self) -> usize {
        Self::idx(self.head, self.len)
    }

    fn tail_inclusive(&self) -> usize {
        assert!(self.len != 0);
        Self::idx(self.head, self.len - 1)
    }

    pub fn get(&self, idx: usize) -> Option<&T> {
        if idx < self.len {
            // SAFETY: We just bounds checked
            Some(unsafe { self.data[Self::idx(self.head, idx)].assume_init_ref() })
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        if idx < self.len {
            // SAFETY: We just bounds checked
            Some(unsafe { self.data[Self::idx(self.head, idx)].assume_init_mut() })
        } else {
            None
        }
    }

    pub fn front(&self) -> Option<&T> {
        self.get(0)
    }

    pub fn back(&self) -> Option<&T> {
        self.get(self.len.wrapping_sub(1))
    }

    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.get_mut(0)
    }

    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.get_mut(self.len.wrapping_sub(1))
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if self.len != 0 {
            // SAFETY: This element is always in-bounds and will no longer be in-bounds after we
            //         return so it cannot be read again.
            let elem = unsafe { self.data[self.head].assume_init_read() };

            if self.head == N - 1 {
                self.head = 0;
            } else {
                self.head += 1;
            }

            Some(elem)
        } else {
            None
        }
    }

    pub fn pop_back(&mut self) -> Option<T> {
        if self.len != 0 {
            // SAFETY: This element is always in-bounds and will no longer be in-bounds after we
            //         return so it cannot be read again.
            let elem = unsafe { self.data[self.tail_inclusive()].assume_init_read() };

            self.len -= 1;
            Some(elem)
        } else {
            None
        }
    }

    pub fn push_front(&mut self, val: T) -> Result<(), T> {
        if self.len == N {
            Err(val)
        } else {
            if self.head == 0 {
                self.head = N - 1;
            } else {
                self.head -= 1;
            }

            self.data[self.head] = MaybeUninit::new(val);
            self.len += 1;
            Ok(())
        }
    }

    pub fn push_back(&mut self, val: T) -> Result<(), T> {
        if self.len == N {
            Err(val)
        } else {
            self.data[self.tail_exclusive()] = MaybeUninit::new(val);
            self.len += 1;
            Ok(())
        }
    }

    pub fn clear(&mut self) {
        self.drain();
        self.head = 0;
    }

    pub fn iter(&self) -> ArrayDequeIter<T, N> {
        ArrayDequeIter(self, self.head, self.len)
    }

    pub fn drain(&mut self) -> ArrayDequeDrain<T, N> {
        ArrayDequeDrain(self)
    }
}

impl<T, const N: usize> Drop for ArrayDeque<T, N> {
    fn drop(&mut self) {
        self.drain();
    }
}

impl<T: fmt::Debug, const N: usize> fmt::Debug for ArrayDeque<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Clone, const N: usize> Clone for ArrayDeque<T, N> {
    fn clone(&self) -> Self {
        let mut new = ArrayDeque::new();

        for val in self.iter() {
            let _ = new.push_back(val.clone());
        }

        new
    }
}

pub struct ArrayDequeIter<'a, T, const N: usize>(&'a ArrayDeque<T, N>, usize, usize);

impl<'a, T, const N: usize> Iterator for ArrayDequeIter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.2 != 0 {
            let item = &self.0.data[self.1];

            if self.1 == N - 1 {
                self.1 = 0;
            } else {
                self.1 += 1;
            }

            self.2 -= 1;

            // SAFETY: This element is always in-bounds since self.1 and self.2 start as the bounds
            //         of the array and only ever shrink while iterating
            Some(unsafe { item.assume_init_ref() })
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0.len, Some(self.0.len))
    }
}

impl<'a, T, const N: usize> DoubleEndedIterator for ArrayDequeIter<'a, T, N> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.2 != 0 {
            self.2 -= 1;

            // SAFETY: This element is always in-bounds since self.1 and self.2 start as the bounds
            //         of the array and only ever shrink while iterating
            Some(unsafe { self.0.data[ArrayDeque::<T, N>::idx(self.1, self.2)].assume_init_ref() })
        } else {
            None
        }
    }
}

pub struct ArrayDequeDrain<'a, T, const N: usize>(&'a mut ArrayDeque<T, N>);

impl<'a, T, const N: usize> Iterator for ArrayDequeDrain<'a, T, N> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.0.pop_front()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0.len, Some(self.0.len))
    }
}

impl<'a, T, const N: usize> DoubleEndedIterator for ArrayDequeDrain<'a, T, N> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.pop_back()
    }
}

impl<'a, T, const N: usize> Drop for ArrayDequeDrain<'a, T, N> {
    fn drop(&mut self) {
        for _ in self {}
    }
}
