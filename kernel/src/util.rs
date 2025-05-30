use alloc::sync::{Arc, Weak};
use core::cell::SyncUnsafeCell;
use core::fmt;
use core::mem::MaybeUninit;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Bound, Deref, DerefMut, Not, RangeBounds, Sub, SubAssign};
use core::pin::Pin;
use core::sync::atomic::{AtomicU8, Ordering};

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

pub struct DebugOrDefault<'a, T: ?Sized>(pub &'a T);

impl<'a, T: ?Sized> fmt::Debug for DebugOrDefault<'a, T> {
    default fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "..")
    }
}

impl<'a, T: ?Sized + fmt::Debug> fmt::Debug for DebugOrDefault<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
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
pub struct SyncPtr<T: ?Sized>(*mut T);
unsafe impl<T: ?Sized> Send for SyncPtr<T> {}
unsafe impl<T: ?Sized> Sync for SyncPtr<T> {}
impl<T: ?Sized> Copy for SyncPtr<T> {}
impl<T: ?Sized> Clone for SyncPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> SyncPtr<T> {
    pub const fn new(ptr: *mut T) -> Self {
        SyncPtr(ptr)
    }

    pub const fn unwrap(self) -> *mut T {
        self.0
    }
}

impl<T: ?Sized> Deref for SyncPtr<T> {
    type Target = *mut T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ?Sized> DerefMut for SyncPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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
    data: [MaybeUninit<T>; N],
}

impl<T, const N: usize> ArrayDeque<T, N> {
    pub fn new() -> Self {
        Self {
            head: 0,
            len: 0,
            data: [const { MaybeUninit::uninit() }; N],
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len == N
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

            self.len -= 1;

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

pub struct OneShotManualInit<T> {
    // 0: Uninitialized
    // 1: Initialization started, but not completed
    // 2: Initialized
    state: AtomicU8,
    val: SyncUnsafeCell<MaybeUninit<T>>,
}

impl<T> OneShotManualInit<T> {
    pub const fn uninit() -> Self {
        Self {
            state: AtomicU8::new(0),
            val: SyncUnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub const fn new(val: T) -> Self {
        Self {
            state: AtomicU8::new(2),
            val: SyncUnsafeCell::new(MaybeUninit::new(val)),
        }
    }

    pub fn is_init(&self) -> bool {
        self.state.load(Ordering::Acquire) == 2
    }

    #[track_caller]
    pub fn set(&self, val: T) -> &T {
        if self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
            panic!("OneShotManualInit initialized multiple times");
        }

        // SAFETY: Since the state was previously 0, nobody else can have any references to val
        //         from before the swap. And since we swap the state with 1, it is not possible for
        //         any other concurrent call to set(...) to get to this point. Therefore, we have
        //         the only reference to the internals of val at this point.
        unsafe {
            (*self.val.get()).write(val);
        }

        self.state.store(2, Ordering::Release);

        // SAFETY: We literally just initialized this
        unsafe { (*self.val.get()).assume_init_ref() }
    }

    pub fn try_get(&self) -> Option<&T> {
        if self.is_init() {
            // SAFETY: Since the state was seen to be 2 above, val must have been fully initialized
            //         and so it is now safe to get a shared reference to it.
            Some(unsafe { (*self.val.get()).assume_init_ref() })
        } else {
            None
        }
    }

    #[track_caller]
    pub fn get(&self) -> &T {
        self.try_get().expect("OneShotManualInit used before being initialized")
    }
}

impl<T> Drop for OneShotManualInit<T> {
    fn drop(&mut self) {
        if self.is_init() {
            // SAFETY: Initialization was complete, so there's definitely a valid value to drop
            //         here.
            unsafe {
                (*self.val.get()).assume_init_drop();
            }
        }
    }
}

pub trait TrueCondition {}
pub trait FalseCondition {}
pub enum Condition<const C: bool> {}

impl TrueCondition for Condition<true> {}
impl FalseCondition for Condition<false> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedBitVector<const N: usize>
where
    [(); N.div_ceil(32)]:,
{
    contents: [u32; N.div_ceil(32)],
}

impl<const N: usize> FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    const INNER_SIZE: usize = N.div_ceil(32);

    pub fn new(default: bool) -> Self {
        Self {
            contents: if default {
                let mut contents = [!0; N.div_ceil(32)];

                let last_bits = N % 32;
                if last_bits != 0 {
                    contents[Self::INNER_SIZE - 1] &= (1 << last_bits) - 1;
                }

                contents
            } else {
                [0; N.div_ceil(32)]
            },
        }
    }

    #[track_caller]
    const fn get_bit_pos(idx: usize) -> (usize, usize) {
        if idx >= N {
            panic!("out of bounds bitvector access");
        }

        (idx >> 5, idx & 31)
    }

    const fn get_idx(idx: usize, bit: usize) -> usize {
        (idx << 5) | bit
    }

    #[track_caller]
    pub const fn get(&self, idx: usize) -> bool {
        let (idx, bit) = Self::get_bit_pos(idx);

        self.contents[idx] & (1 << bit) != 0
    }

    #[track_caller]
    pub const fn set(&mut self, idx: usize, val: bool) -> bool {
        let (idx, bit) = Self::get_bit_pos(idx);

        let old_val = self.contents[idx] & (1 << bit) != 0;

        if val {
            self.contents[idx] |= 1 << bit;
        } else {
            self.contents[idx] &= !(1 << bit);
        }

        old_val
    }

    #[track_caller]
    pub fn set_range(&mut self, idx: impl RangeBounds<usize>, val: bool) {
        let start = match idx.start_bound() {
            Bound::Excluded(&start) if start == !0 => {
                return;
            },
            Bound::Excluded(&start) => start + 1,
            Bound::Included(&start) => start,
            Bound::Unbounded => 0,
        };

        let end = match idx.end_bound() {
            Bound::Excluded(&0) => {
                return;
            },
            Bound::Excluded(&end) => end - 1,
            Bound::Included(&end) => end,
            Bound::Unbounded => N - 1,
        };

        if end <= start {
            return;
        }

        let (start_idx, start_bit) = Self::get_bit_pos(start);
        let (end_idx, end_bit) = Self::get_bit_pos(end);

        for idx in start_idx..=end_idx {
            let mut mask = !0;

            if idx == start_idx && start_bit != 0 {
                mask &= !((1 << start_bit) - 1);
            }

            if idx == end_idx && end_bit != 31 {
                mask &= (1 << (end_bit + 1)) - 1
            }

            if val {
                self.contents[idx] |= mask;
            } else {
                self.contents[idx] &= !mask;
            }
        }
    }

    pub fn invert(&mut self) {
        for word in self.contents.iter_mut() {
            *word ^= !0;
        }

        let last_bits = N % 32;
        if last_bits != 0 {
            self.contents[Self::INNER_SIZE - 1] &= (1 << last_bits) - 1;
        }
    }

    pub fn count(&self) -> usize {
        let mut count = 0;

        for word in self.contents.iter() {
            count += word.count_ones() as usize;
        }

        count
    }

    pub fn iter(&self) -> FixedBitVectorIter<N> {
        FixedBitVectorIter(self, 0)
    }

    pub fn find_next(&self, start: usize) -> Option<usize> {
        if start >= N {
            return None;
        }

        let (mut idx, start_bit) = Self::get_bit_pos(start);
        let mut mask = !((1 << start_bit) - 1);

        while idx < Self::INNER_SIZE {
            let word_val = self.contents[idx] & mask;

            if word_val != 0 {
                return Some(Self::get_idx(idx, word_val.trailing_zeros() as usize));
            }

            idx += 1;
            mask = !0;
        }

        None
    }

    pub fn resize<const M: usize>(self) -> FixedBitVector<M>
    where
        [(); M.div_ceil(32)]:,
    {
        let mut contents = [0; M.div_ceil(32)];

        if M >= N {
            contents[..N.div_ceil(32)].clone_from_slice(&self.contents);
        } else {
            contents.clone_from_slice(&self.contents[..M.div_ceil(32)]);

            let last_bits = M % 32;
            if last_bits != 0 {
                contents[FixedBitVector::<M>::INNER_SIZE - 1] &= (1 << last_bits) - 1;
            }
        }

        FixedBitVector { contents }
    }

    pub fn grow<const M: usize>(self) -> FixedBitVector<M>
    where
        [(); M.div_ceil(32)]:,
        Condition<{ M >= N }>: TrueCondition,
    {
        self.resize()
    }

    pub fn shrink<const M: usize>(self) -> FixedBitVector<M>
    where
        [(); M.div_ceil(32)]:,
        Condition<{ M <= N }>: TrueCondition,
    {
        self.resize()
    }
}

impl<const N: usize> BitAndAssign for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    fn bitand_assign(&mut self, rhs: Self) {
        for i in 0..Self::INNER_SIZE {
            self.contents[i] &= rhs.contents[i];
        }
    }
}

impl<const N: usize> BitAnd for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    type Output = FixedBitVector<N>;

    fn bitand(self, rhs: Self) -> Self::Output {
        let mut result = self;
        result &= rhs;

        result
    }
}

impl<const N: usize> BitOrAssign for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    fn bitor_assign(&mut self, rhs: Self) {
        for i in 0..Self::INNER_SIZE {
            self.contents[i] |= rhs.contents[i];
        }
    }
}

impl<const N: usize> BitOr for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    type Output = FixedBitVector<N>;

    fn bitor(self, rhs: Self) -> Self::Output {
        let mut result = self;
        result |= rhs;

        result
    }
}

impl<const N: usize> SubAssign for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    fn sub_assign(&mut self, rhs: Self) {
        for i in 0..Self::INNER_SIZE {
            self.contents[i] &= !rhs.contents[i];
        }
    }
}

impl<const N: usize> Sub for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    type Output = FixedBitVector<N>;

    fn sub(self, rhs: Self) -> Self::Output {
        let mut result = self;
        result -= rhs;

        result
    }
}

impl<const N: usize> Not for FixedBitVector<N>
where
    [(); N.div_ceil(32)]:,
{
    type Output = FixedBitVector<N>;

    fn not(self) -> Self::Output {
        let mut result = self;
        result.invert();

        result
    }
}

pub struct FixedBitVectorIter<'a, const N: usize>(&'a FixedBitVector<N>, usize)
where
    [(); N.div_ceil(32)]:;

impl<'a, const N: usize> Iterator for FixedBitVectorIter<'a, N>
where
    [(); N.div_ceil(32)]:,
{
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let FixedBitVectorIter(bv, ref mut start) = *self;

        if let Some(idx) = bv.find_next(*start) {
            *start = idx + 1;
            Some(idx)
        } else {
            *start = N;
            None
        }
    }
}

#[cfg(test)]
mod test {
    use super::{ArrayDeque, FixedBitVector};

    #[test_case]
    fn test_array_deque_new() {
        let a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(0, a.len());
        assert_eq!(None, a.get(0));
    }

    #[test_case]
    fn test_array_deque_push_back() {
        let mut a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(Ok(()), a.push_back(1234));

        assert_eq!(1, a.len());
        assert_eq!(Some(&1234), a.get(0));

        assert_eq!(Ok(()), a.push_back(5678));

        assert_eq!(2, a.len());
        assert_eq!(Some(&1234), a.get(0));
        assert_eq!(Some(&5678), a.get(1));
    }

    #[test_case]
    fn test_array_deque_push_back_full() {
        let mut a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(Ok(()), a.push_back(0));
        assert_eq!(Ok(()), a.push_back(0));
        assert_eq!(Ok(()), a.push_back(0));
        assert_eq!(Ok(()), a.push_back(0));
        assert_eq!(Err(1234), a.push_back(1234));

        assert_eq!(4, a.len());
        assert_eq!(Some(&0), a.get(0));
        assert_eq!(Some(&0), a.get(3));
    }

    #[test_case]
    fn test_array_deque_push_front() {
        let mut a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(Ok(()), a.push_front(1234));

        assert_eq!(1, a.len());
        assert_eq!(Some(&1234), a.get(0));

        assert_eq!(Ok(()), a.push_front(5678));

        assert_eq!(2, a.len());
        assert_eq!(Some(&5678), a.get(0));
        assert_eq!(Some(&1234), a.get(1));
    }

    #[test_case]
    fn test_array_deque_push_front_full() {
        let mut a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(Ok(()), a.push_front(0));
        assert_eq!(Ok(()), a.push_front(0));
        assert_eq!(Ok(()), a.push_front(0));
        assert_eq!(Ok(()), a.push_front(0));
        assert_eq!(Err(1234), a.push_front(1234));

        assert_eq!(4, a.len());
        assert_eq!(Some(&0), a.get(0));
        assert_eq!(Some(&0), a.get(3));
    }

    #[test_case]
    fn test_array_deque_pop_front() {
        let mut a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(None, a.pop_front());
        assert_eq!(0, a.len());

        assert_eq!(Ok(()), a.push_back(1234));
        assert_eq!(Ok(()), a.push_back(5678));

        assert_eq!(Some(1234), a.pop_front());
        assert_eq!(1, a.len());

        assert_eq!(Some(5678), a.pop_front());
        assert_eq!(0, a.len());

        assert_eq!(None, a.pop_front());
        assert_eq!(0, a.len());
    }

    #[test_case]
    fn test_array_deque_pop_back() {
        let mut a: ArrayDeque<u32, 4> = ArrayDeque::new();

        assert_eq!(None, a.pop_front());
        assert_eq!(0, a.len());

        assert_eq!(Ok(()), a.push_back(1234));
        assert_eq!(Ok(()), a.push_back(5678));

        assert_eq!(Some(5678), a.pop_back());
        assert_eq!(1, a.len());

        assert_eq!(Some(1234), a.pop_back());
        assert_eq!(0, a.len());

        assert_eq!(None, a.pop_back());
        assert_eq!(0, a.len());
    }

    #[test_case]
    fn test_fixed_bv_init() {
        let fbv = FixedBitVector::<32>::new(false);
        assert_eq!(fbv.contents, [0]);

        let fbv = FixedBitVector::<33>::new(false);
        assert_eq!(fbv.contents, [0, 0]);

        let fbv = FixedBitVector::<32>::new(true);
        assert_eq!(fbv.contents, [!0]);

        let fbv = FixedBitVector::<33>::new(true);
        assert_eq!(fbv.contents, [!0, 1]);
    }

    #[test_case]
    fn test_fixed_bv_get() {
        assert!(!FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        }
        .get(0));
        assert!(FixedBitVector::<48> {
            contents: [0x0000_0001, 0x0000_0000]
        }
        .get(0));

        assert!(!FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        }
        .get(31));
        assert!(FixedBitVector::<48> {
            contents: [0x8000_0000, 0x0000_0000]
        }
        .get(31));

        assert!(!FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        }
        .get(32));
        assert!(FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0001]
        }
        .get(32));

        assert!(!FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        }
        .get(47));
        assert!(FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_8000]
        }
        .get(47));
    }

    #[test_case]
    fn test_fixed_bv_set() {
        let mut fbv = FixedBitVector::<48>::new(false);

        assert!(!fbv.set(0, true));
        assert_eq!(fbv.contents, [1, 0]);

        assert!(fbv.set(0, true));
        assert_eq!(fbv.contents, [1, 0]);

        assert!(fbv.set(0, false));
        assert_eq!(fbv.contents, [0, 0]);

        assert!(!fbv.set(0, false));
        assert_eq!(fbv.contents, [0, 0]);

        assert!(!fbv.set(31, true));
        assert_eq!(fbv.contents, [0x8000_0000, 0]);

        assert!(fbv.set(31, true));
        assert_eq!(fbv.contents, [0x8000_0000, 0]);

        assert!(fbv.set(31, false));
        assert_eq!(fbv.contents, [0, 0]);

        assert!(!fbv.set(31, false));
        assert_eq!(fbv.contents, [0, 0]);

        assert!(!fbv.set(32, true));
        assert_eq!(fbv.contents, [0, 1]);

        assert!(fbv.set(32, true));
        assert_eq!(fbv.contents, [0, 1]);

        assert!(!fbv.set(47, true));
        assert_eq!(fbv.contents, [0, 0x8001]);

        assert!(fbv.set(47, true));
        assert_eq!(fbv.contents, [0, 0x8001]);

        assert!(fbv.set(32, false));
        assert_eq!(fbv.contents, [0, 0x8000]);

        assert!(!fbv.set(32, false));
        assert_eq!(fbv.contents, [0, 0x8000]);

        assert!(fbv.set(47, false));
        assert_eq!(fbv.contents, [0, 0]);

        assert!(!fbv.set(47, false));
        assert_eq!(fbv.contents, [0, 0]);
    }

    #[test_case]
    fn test_fixed_bv_set_range() {
        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(0..0, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(!0..0, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(0..48, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ffff, 0x0000_ffff]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(0..40, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ffff, 0x0000_00ff]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(0..32, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ffff, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(8..48, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ff00, 0x0000_ffff]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(8..40, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ff00, 0x0000_00ff]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(8..32, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ff00, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(false);
        fbv.set_range(8..24, true);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x00ff_ff00, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(0..0, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ffff, 0x0000_ffff]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(!0..0, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xffff_ffff, 0x0000_ffff]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(0..48, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(0..40, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_ff00]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(0..32, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_0000, 0x0000_ffff]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(8..48, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_00ff, 0x0000_0000]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(8..40, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_00ff, 0x0000_ff00]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(8..32, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0x0000_00ff, 0x0000_ffff]
        });

        let mut fbv = FixedBitVector::<48>::new(true);
        fbv.set_range(8..24, false);
        assert_eq!(fbv, FixedBitVector::<48> {
            contents: [0xff00_00ff, 0x0000_ffff]
        });
    }

    #[test_case]
    fn test_fixed_bv_count() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            }
            .count(),
            0
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8000_0000, 0x0000_0000]
            }
            .count(),
            1
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_8000]
            }
            .count(),
            1
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x1111_1111, 0x0000_1111]
            }
            .count(),
            12
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            }
            .count(),
            48
        );
    }

    #[test_case]
    fn test_fixed_bv_find_next() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            }
            .find_next(0),
            None
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0001, 0x0000_0000]
            }
            .find_next(0),
            Some(0)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0001, 0x0000_0000]
            }
            .find_next(1),
            None
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0003, 0x0000_0000]
            }
            .find_next(1),
            Some(1)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8000_0000, 0x0000_0000]
            }
            .find_next(0),
            Some(31)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8000_0000, 0x0000_ffff]
            }
            .find_next(0),
            Some(31)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8000_0000, 0x0000_0000]
            }
            .find_next(32),
            None
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_8000]
            }
            .find_next(0),
            Some(47)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8000_0000, 0x0000_8000]
            }
            .find_next(32),
            Some(47)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0001, 0x0000_8000]
            }
            .find_next(1),
            Some(47)
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            }
            .find_next(48),
            None
        );

        assert_eq!(FixedBitVector::<32> { contents: [0xffff_ffff] }.find_next(32), None);
    }

    #[test_case]
    fn test_fixed_bv_not() {
        assert_eq!(
            !FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            }
        );

        assert_eq!(
            !FixedBitVector::<48> {
                contents: [0x1248_3000, 0x0000_8001]
            },
            FixedBitVector::<48> {
                contents: [0xedb7_cfff, 0x0000_7ffe]
            }
        );
    }

    #[test_case]
    fn test_fixed_bv_and() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0xf0f0_f0f0, 0x0000_0f0f]
            } & FixedBitVector::<48> {
                contents: [0x0f0f_0f0f, 0x0000_f0f0]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_00ff]
            },
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_ffff]
            } & FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_00ff]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x1248_edb7, 0x0000_f0f0]
            },
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            } & FixedBitVector::<48> {
                contents: [0x1248_edb7, 0x0000_f0f0]
            }
        );
    }

    #[test_case]
    fn test_fixed_bv_or() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            } | FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_ffff, 0x0000_ffff]
            },
            FixedBitVector::<48> {
                contents: [0x0000_ffff, 0x0000_0000]
            } | FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_ffff]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_ffff, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0x0000_0fff, 0x0000_0000]
            } | FixedBitVector::<48> {
                contents: [0x0000_fff0, 0x0000_0000]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_9669, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0x0000_1248, 0x0000_0000]
            } | FixedBitVector::<48> {
                contents: [0x0000_8421, 0x0000_0000]
            }
        );
    }

    #[test_case]
    fn test_fixed_bv_diff() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            } - FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            } - FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            } - FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            },
            FixedBitVector::<48> {
                contents: [0xffff_ffff, 0x0000_ffff]
            } - FixedBitVector::<48> {
                contents: [0x0000_0000, 0x0000_0000]
            }
        );

        assert_eq!(
            FixedBitVector::<48> {
                contents: [0xedb7_6537, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0xffff_7777, 0x0000_0000]
            } - FixedBitVector::<48> {
                contents: [0x1248_1248, 0x0000_1248]
            }
        );
    }

    #[test_case]
    fn test_fixed_bv_grow() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .grow::<48>()
        );

        assert_eq!(
            FixedBitVector::<64> {
                contents: [0x8765_4321, 0x0000_cba9]
            },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .grow::<64>()
        );

        assert_eq!(
            FixedBitVector::<96> {
                contents: [0x8765_4321, 0x0000_cba9, 0x0000_0000]
            },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .grow::<96>()
        );
    }

    #[test_case]
    fn test_fixed_bv_shrink() {
        assert_eq!(
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .shrink::<48>()
        );

        assert_eq!(
            FixedBitVector::<40> {
                contents: [0x8765_4321, 0x0000_00a9]
            },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .shrink::<40>()
        );

        assert_eq!(
            FixedBitVector::<32> { contents: [0x8765_4321] },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .shrink::<32>()
        );

        assert_eq!(
            FixedBitVector::<16> { contents: [0x0000_4321] },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .shrink::<16>()
        );

        assert_eq!(
            FixedBitVector::<0> { contents: [] },
            FixedBitVector::<48> {
                contents: [0x8765_4321, 0x0000_cba9]
            }
            .shrink::<0>()
        );
    }
}
