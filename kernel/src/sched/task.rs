//! Data structures used by the scheduler to track processes and threads.

use core::cell::UnsafeCell;
use core::fmt;
use core::marker::PhantomData;
use core::mem::{self, MaybeUninit};
use core::pin::Pin;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::boxed::Box;
use alloc::sync::Arc;

use super::wait::ThreadWaitState;
use crate::sync::uninterruptible::{InterruptDisabler, UninterruptibleSpinlock, UninterruptibleSpinlockGuard};
use crate::util::{PinWeak, SharedUnsafeCell};
use crate::x86_64::idt::InterruptFrame;
use crate::x86_64::regs::SavedRegisters;

static NEXT_PID: AtomicU64 = AtomicU64::new(0);

static KERNEL_PROCESS: SharedUnsafeCell<MaybeUninit<Pin<Arc<Process>>>> = SharedUnsafeCell::new(MaybeUninit::uninit());

struct ProcessInternal {
    next_thread_id: u64,
    threads_head: Option<Pin<Arc<Thread>>>,
    threads_tail: *const Thread,
    ready_head: *const Thread,
    ready_tail: *const Thread
}

unsafe impl Send for ProcessInternal {}
impl !Unpin for ProcessInternal {}

/// A structure containing state information for a process.
///
/// The lifetime of this structure is managed internally by the scheduler so that pointers to a process can be safely stored in other
/// scheduler data structures. All values of this type must live inside of a pinned [`Arc`], and all methods of creating processes will
/// ensure this.
pub struct Process {
    pid: u64,
    internal: UninterruptibleSpinlock<ProcessInternal>
}

impl Process {
    fn create_internal(pid: u64) -> Pin<Arc<Process>> {
        Arc::pin(Process {
            pid,
            internal: UninterruptibleSpinlock::new(ProcessInternal {
                next_thread_id: 0,
                threads_head: None,
                threads_tail: ptr::null(),
                ready_head: ptr::null(),
                ready_tail: ptr::null()
            })
        })
    }

    /// Initializes the kernel process and its init thread.
    ///
    /// # Safety
    ///
    /// This method must only be called once during startup from the bootstrap processor. This should be called early during the startup
    /// process, as calling [`Process::kernel`] is technically unsafe until this method is called.
    pub(super) unsafe fn init_kernel_process() {
        debug_assert!(NEXT_PID.load(Ordering::Relaxed) == 0);

        ptr::write((*KERNEL_PROCESS.get()).as_mut_ptr(), Process::create_internal(0));
        NEXT_PID.store(1, Ordering::Relaxed);

        let init_thread = Thread::create_internal(&mut Process::kernel().lock(), SavedRegisters::new());
        init_thread.lock().guard.state = ThreadState::Running;
        *CURRENT_THREAD.get() = Some(init_thread);
    }

    /// Gets a reference to the kernel process.
    pub fn kernel() -> &'static Pin<Arc<Process>> {
        // SAFETY: KERNEL_PROCESS is only set once during initialization, so it is safe to get shared references to it once initialization
        //         is complete.
        unsafe {
            debug_assert!(NEXT_PID.load(Ordering::Relaxed) > 0);
            &*(*KERNEL_PROCESS.get()).as_ptr()
        }
    }

    /// Gets this process's PID.
    pub fn pid(&self) -> u64 {
        self.pid
    }

    /// Checks whether this process is the kernel process.
    pub fn is_kernel_process(&self) -> bool {
        self.pid == 0
    }

    /// Locks this process's mutable state.
    ///
    /// # Lock Ordering
    ///
    /// In general, the only other scheduler lock that is safe to hold while calling this method is the lock on the list of processes.
    ///
    /// This method must not be called while any other processes or threads are locked by the current core. Doing so could result in a
    /// deadlock occurring.
    pub fn lock(&self) -> ProcessLock {
        ProcessLock {
            guard: self.internal.lock(),
            process: self
        }
    }

    fn as_arc(&self) -> Pin<Arc<Process>> {
        // SAFETY: All processes must be in an Arc. This is true since the only way to create a process is via Process::create_internal,
        //         which returns a Pin<Arc<Process>>. Since processes created in this way must be in an Arc and cannot be moved out due to
        //         being in a Pin, any valid &Process must be allocated in an Arc.
        unsafe {
            Arc::increment_strong_count(self);
            Pin::new_unchecked(Arc::from_raw(self))
        }
    }
}

/// A lock guard providing access to a process's mutable state.
///
/// # Interrupts
///
/// Processor interrupts are automatically disabled on the current core while a process lock is held to allow for context switching from
/// within interrupt handlers. For this reason, critical sections holding such locks should be as short as reasonably possible.
pub struct ProcessLock<'a> {
    guard: UninterruptibleSpinlockGuard<'a, ProcessInternal>,
    process: &'a Process
}

impl <'a> ProcessLock<'a> {
    /// Gets an iterator that returns all threads belonging to this process.
    pub fn threads<'b>(&'b self) -> impl Iterator<Item=Pin<Arc<Thread>>> + 'b {
        ProcessThreadIterator(self.guard.threads_head.clone(), PhantomData)
    }

    fn create_kernel_thread_internal(&mut self, f: extern "C" fn (*mut u8) -> !, arg: *mut u8, stack_size: usize) -> Pin<Arc<Thread>> {
        let stack = crate::early_alloc::alloc(stack_size, 16); // TODO Allocate pages instead. Place guard page.
        Thread::create_internal(self, SavedRegisters::new_kernel_thread(f, arg, unsafe { stack.add(stack_size) }))
    }

    /// Creates a new kernel-mode thread in this process that executes the provided function. The stack of the new thread will be at least
    /// `stack_size` bytes large.
    ///
    /// # Panics
    ///
    /// This method can only be used on the kernel process. For safety reasons, creating kernel-mode threads in user-space processes is not
    /// allowed and attempting to do so will cause a panic.
    pub fn create_kernel_thread<F: FnOnce () -> ()>(&mut self, f: F, stack_size: usize) -> Pin<Arc<Thread>> {
        extern "C" fn run<F: FnOnce () -> ()>(ptr: *mut u8) -> ! {
            unsafe {
                let f = *Box::from_raw(ptr as *mut F);

                f();
                Thread::kill_current();
            };
        }

        assert!(self.process.is_kernel_process());
        self.create_kernel_thread_internal(run::<F>, Box::into_raw(Box::new(f)) as *mut u8, stack_size)
    }

    /// Creates a new user-mode thread in this process that executes a function at the provided user-mode address. The stack of the new
    /// thread will be at least `stack_size` bytes large.
    ///
    /// # Panics
    ///
    /// This method cannot be used on the kernel process and attempting to do so will cause a panic.
    pub fn create_user_thread(&mut self, f: u64, arg: u64, stack_size: usize) -> Pin<Arc<Thread>> {
        assert!(!self.process.is_kernel_process());

        // TODO Actually allocate a user-mode stack
        Thread::create_internal(self, SavedRegisters::new_user_thread(f, arg, 0))
    }

    unsafe fn remove_thread(&mut self, thread: &Pin<Arc<Thread>>) {
        debug_assert_eq!(self.process as *const _, thread.process.as_ptr());

        let process_internal = &mut *thread.process_internal.get();

        if let Some(ref mut next) = process_internal.next {
            (*next.process_internal.get()).prev = process_internal.prev;
        } else {
            self.guard.threads_tail = process_internal.prev;
        };

        if !process_internal.prev.is_null() {
            (*(*process_internal.prev).process_internal.get()).next = process_internal.next.take();
        } else {
            self.guard.threads_head = process_internal.next.take();
        };
    }

    /// Attempts to dequeue a thread from this process's queue of threads that are in the ready state. If this process does not have any
    /// threads in the ready state, returns [`None`].
    pub(super) fn dequeue_ready_thread(&mut self) -> Option<&Thread> {
        if !self.guard.ready_head.is_null() {
            // SAFETY: Since we have locked the process owning these threads, we have also conceptually locked their ThreadProcessInternal
            //         data. So long as the ready list is in a valid state, dequeueing a thread from it is perfectly safe.
            unsafe {
                let thread = &*self.guard.ready_head;
                let process_internal = &mut *thread.process_internal.get();

                self.guard.ready_head = if !process_internal.next_ready.is_null() {
                    (*(*process_internal.next_ready).process_internal.get()).prev_ready = ptr::null();
                    process_internal.next_ready
                } else {
                    self.guard.ready_tail = ptr::null();
                    ptr::null()
                };

                process_internal.prev_ready = ptr::null();
                process_internal.next_ready = ptr::null();

                Some(thread)
            }
        } else {
            None
        }
    }

    /// Enqueues the provided thread on this process's queue of threads that are in the ready state.
    ///
    /// # Safety
    ///
    /// The provided thread must belong to this process, must be in the ready state, and must not have already been placed on the queue of
    /// ready threads.
    pub(super) unsafe fn enqueue_ready_thread(&mut self, thread_lock: ThreadLock) {
        let thread = thread_lock.thread;

        debug_assert_eq!(self.process as *const _, thread.process.as_ptr());
        debug_assert!(matches!(thread_lock.guard.state, ThreadState::Ready));
        debug_assert!((*thread_lock.thread.process_internal.get()).next_ready.is_null());
        debug_assert!(!ptr::eq(self.guard.ready_tail, thread));

        mem::drop(thread_lock);

        let process_internal = &mut *thread.process_internal.get();

        process_internal.next_ready = ptr::null();
        if !self.guard.ready_tail.is_null() {
            process_internal.prev_ready = self.guard.ready_tail;
            (*(*self.guard.ready_tail).process_internal.get()).next_ready = thread as *const _;
        } else {
            process_internal.prev_ready = ptr::null();
            self.guard.ready_head = thread as *const _;
        };
        self.guard.ready_tail = thread as *const _;
    }

    /// Gets a reference to the Process structure that this guard has locked.
    pub fn process(&self) -> &'a Process {
        self.process
    }
}

struct ProcessThreadIterator<'a, 'b>(Option<Pin<Arc<Thread>>>, PhantomData<&'a ProcessLock<'b>>);

impl <'a, 'b> Iterator for ProcessThreadIterator<'a, 'b> {
    type Item = Pin<Arc<Thread>>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(thread) = self.0.take() {
            // SAFETY: Conceptually, the process owns its threads' ThreadProcessInternal data
            self.0 = unsafe { (*thread.process_internal.get()).next.clone() };
            Some(thread)
        } else {
            None
        }
    }
}

/// Represents the execution state of a thread.
#[derive(Debug)]
pub enum ThreadState {
    /// The thread is currently suspended and can be resumed by calling [`ThreadLock::wake`].
    Suspended,
    /// The thread is currently waiting on a [`ThreadWaitList`](super::wait::ThreadWaitList).
    Waiting(ThreadWaitState),
    /// The thread is ready to run, but is not currently scheduled on a CPU core.
    Ready,
    /// The thread is currently running on a CPU core.
    Running,
    /// The thread has been terminated and its resources have been freed.
    Dead
}

struct ThreadInternal {
    state: ThreadState,
    regs: SavedRegisters
}

unsafe impl Send for ThreadInternal {}

struct ThreadProcessInternal {
    prev: *const Thread,
    next: Option<Pin<Arc<Thread>>>,
    prev_ready: *const Thread,
    next_ready: *const Thread
}

unsafe impl Send for ThreadProcessInternal {}

#[thread_local]
static CURRENT_THREAD: UnsafeCell<Option<Pin<Arc<Thread>>>> = UnsafeCell::new(None);

/// A structure containing state information for a thread.
///
/// The lifetime of this structure is managed internally by the scheduler so that pointers to a thread can be safely stored in other
/// scheduler data structures. All values of this type must live inside of a pinned [`Arc`], and all methods of creating threads will ensure
/// this.
pub struct Thread {
    process: PinWeak<Process>,
    thread_id: u64,
    internal: UninterruptibleSpinlock<ThreadInternal>,
    process_internal: SharedUnsafeCell<ThreadProcessInternal>
}

impl !Unpin for Thread {}

impl Thread {
    fn create_internal(process_lock: &mut ProcessLock, regs: SavedRegisters) -> Pin<Arc<Thread>> {
        let thread = Arc::pin(Thread {
            process: PinWeak::downgrade(&process_lock.process.as_arc()),
            thread_id: process_lock.guard.next_thread_id,
            internal: UninterruptibleSpinlock::new(ThreadInternal {
                state: ThreadState::Suspended,
                regs
            }),
            process_internal: SharedUnsafeCell::new(ThreadProcessInternal {
                prev: process_lock.guard.threads_tail,
                next: None,
                prev_ready: ptr::null(),
                next_ready: ptr::null()
            })
        });

        process_lock.guard.next_thread_id += 1;
        process_lock.guard.threads_tail = &*thread;

        if process_lock.guard.threads_head.is_none() {
            process_lock.guard.threads_head = Some(thread.clone());
        } else {
            // SAFETY: Conceptually, the process owns its threads' ThreadProcessInternal data
            unsafe {
                (*(*process_lock.guard.threads_tail).process_internal.get()).next = Some(thread.clone());
            };
        };

        thread
    }

    /// Gets the thread that was executing on the current core before an interrupt occurred. If the idle thread was executing, this method
    /// will return [`None`]. If an interrupt is not currently being handled, then this method will return the currently executing thread.
    pub fn current_interrupted() -> Option<Pin<Arc<Thread>>> {
        // SAFETY: CURRENT_THREAD is thread-local and no references to it ever escape this module
        unsafe {
            (*CURRENT_THREAD.get()).clone()
        }
    }

    /// Gets the thread that is executing on the current core.
    ///
    /// # Panics
    ///
    /// This method will panic if an asynchronous hardware interrupt is currently being handled, as it is not generally safe to assume that
    /// a thread must be running when an interrupt occurs. [`Thread::current_interrupted`] should be used instead of this for code that may
    /// execute in the context of an asynchronous interrupt handler.
    pub fn current() -> Pin<Arc<Thread>> {
        assert!(!super::is_handling_interrupt());
        Thread::current_interrupted().unwrap()
    }

    /// Suspends the currently executing thread and invokes a context switch to another ready thread.
    ///
    /// # Panics
    ///
    /// This method will panic if any [`InterruptDisabler`](InterruptDisabler) values currently exist on this thread, aside from the one
    /// held in the thread lock passed into this method. Context switching while an uninterruptible lock guard is held could result in a
    /// deadlock due to the new thread trying to acquire a lock that was held prior to a context switch.
    ///
    /// # Safety
    ///
    /// The provided thread lock must correspond to the currently executing thread, which must be set to a non-running state prior to
    /// calling this method.
    pub unsafe fn suspend_current(thread_lock: ThreadLock) {
        assert!(ptr::eq(&**(*CURRENT_THREAD.get()).as_ref().unwrap(), thread_lock.thread));
        debug_assert!(!matches!(*thread_lock.state(), ThreadState::Running));

        if InterruptDisabler::num_held() != 1 {
            panic!("Attempt to call Thread::suspend_thread with live InterruptDisabler");
        };

        todo!() // Will need a special interrupt for this to allow a context switch into another thread.
    }

    /// Kills the current thread and ends execution immediately. All kernel-mode stack memory and other scheduler managed resources used by
    /// this thread will be freed immediately.
    ///
    /// # Safety
    ///
    /// This method must not be called while handling an asynchronous hardware interrupt. It may only be called from the context of code
    /// that is considered to run within the thread itself.
    ///
    /// This method does not run Drop implementations for any objects on the stack, so callers must be careful to ensure that there do not
    /// exist any resources on the stack that require explicit dropping when calling this method.
    pub unsafe fn kill_current() -> ! {
        let thread = Thread::current();
        let process = thread.process().upgrade().unwrap();

        let mut process_lock = process.lock();
        let mut thread_lock = thread.lock();

        debug_assert!(matches!(*thread_lock.state(), ThreadState::Running));
        *thread_lock.state_mut() = ThreadState::Dead;
        process_lock.remove_thread(&thread);

        mem::drop(process_lock);
        Thread::suspend_current(thread_lock);
        panic!("Dead thread was resurrected");
    }

    /// Gets a reference to the process in which this thread is running.
    ///
    /// The returned weak reference will always be present so long as this thread is not dead. In the event that this thread is dead, the
    /// weak reference may no longer be present if the process was terminated while a reference to this thread was outstanding.
    pub fn process(&self) -> &PinWeak<Process> {
        &self.process
    }

    /// Gets the thread ID of this thread.
    pub fn thread_id(&self) -> u64 {
        self.thread_id
    }

    /// Locks this thread's mutable state.
    ///
    /// # Lock Ordering
    ///
    /// This method may be called while holding the lock of the process in which it exists or the lock of a wait queue on which this thread
    /// is waiting, as well as any locks that were held when such a lock was acquired.
    ///
    /// This method must not be called while any other threads are locked by the current core. Doing so could result in a deadlock
    /// occurring.
    pub fn lock(&self) -> ThreadLock {
        ThreadLock {
            guard: self.internal.lock(),
            thread: self
        }
    }

    /// Gets a unique identifiable name for this thread for use in kernel debug messages. This name is meant to be human-readable and is not
    /// guaranteed to remain exactly the same throughout the thread's lifecycle.
    pub fn debug_name<'a>(&'a self) -> impl fmt::Display + 'a {
        ThreadDebugName(self)
    }
}

struct ThreadDebugName<'a>(&'a Thread);

impl fmt::Display for ThreadDebugName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let thread = self.0;

        if let Some(process) = thread.process.upgrade() {
            write!(f, "pid {}, thread {}", process.pid(), thread.thread_id())
        } else {
            write!(f, "(disconnected thread {:p})", thread)
        }
    }
}

/// A lock guard providing access to a process's mutable state.
///
/// # Interrupts
///
/// Processor interrupts are automatically disabled on the current core while a thread lock is held to allow for context switching from
/// within interrupt handlers. For this reason, critical sections holding such locks should be as short as reasonably possible.
pub struct ThreadLock<'a> {
    guard: UninterruptibleSpinlockGuard<'a, ThreadInternal>,
    thread: &'a Thread
}

impl <'a> ThreadLock<'a> {
    /// Gets a reference to the current state of this thread.
    pub fn state(&self) -> &ThreadState {
        &self.guard.state
    }

    /// Gets a mutable reference to the current state of this thread.
    ///
    /// # Safety
    ///
    /// Modifying the state of a thread incorrectly can result in undefined behaviour in other parts of the kernel. Care must be taken to
    /// avoid scenarios including, but not limited to, the following:
    ///
    /// - Waking a thread without removing it from a wait queue on which it is currently waiting
    /// - Waking a thread that is waiting uninterruptibly unless the event it is waiting on has occurred
    /// - Marking a thread that is executing kernel code as dead
    /// - Modifying the state of a thread that is currently running on another CPU core
    ///
    /// This method only sets the state of this thread and does not update any other related data structures, e.g. wait lists, ready queues,
    /// etc. Any such data structures must be updated either before calling this method or between calling this method and releasing this
    /// thread lock.
    ///
    /// Care must also be taken when modifying the state of the currently executing thread. Hardware interrupts and threads running on other
    /// CPU cores may assume that the thread has been correctly suspended if it is not marked as being in the running state. If the thread
    /// state of the currently running thread is set to anything other than running using the returned mutable reference, the thread must be
    /// correctly suspended before this lock is released. As long as an asynchronous hardware interrupt is not currently being handled, this
    /// can be done by calling [`Thread::suspend_current`] and passing this thread lock as the argument.
    pub(super) unsafe fn state_mut(&mut self) -> &mut ThreadState {
        &mut self.guard.state
    }

    /// Saves the CPU state of a thread in preparation to potentially perform a context switch.
    ///
    /// # Safety
    ///
    /// The provided interrupt frame must correspond to an interrupt that is being handled on the current core whose contents represent the
    /// real state of this thread at the time the interrupt occurred.
    pub(super) unsafe fn save_cpu_state(&mut self, interrupt_frame: &InterruptFrame) {
        let regs = self.regs_mut();

        interrupt_frame.save(&mut regs.basic);
        regs.ext.save();
    }

    /// Restores the CPU state of a thread so that it will run it once the current interrupt is finished.
    ///
    /// # Safety
    ///
    /// The provided interrupt frame must correspond to an interrupt that is being handled on the current core whose contents will be
    /// restored once the interrupt handler returns. The currently executing thread (if there is any) must have been properly suspended or
    /// killed, and the state of that thread must have been saved using [`ThreadLock::save_cpu_state`] if it may resume execution later.
    ///
    /// If this thread really will resume execution on the current core, then it must be manually updated to be in the running state _before
    /// this lock is released_. If this lock is released without marking the thread as running, the restored state could become stale as
    /// other CPU cores may update it and expect the changes to be reflected when the thread is next resumed.
    pub(super) unsafe fn restore_cpu_state(&self, interrupt_frame: &mut InterruptFrame) {
        let regs = self.regs();

        interrupt_frame.restore(&regs.basic);
        regs.ext.restore();
    }

    /// Gets a reference to the [`Thread`] structure that this guard has locked.
    pub fn thread(&self) -> &'a Thread {
        self.thread
    }

    /// Wakes this thread up from a suspended state and moves it to the ready state.
    pub fn wake(mut self) {
        assert!(matches!(self.guard.state, ThreadState::Suspended));

        self.guard.state = ThreadState::Ready;

        unsafe {
            self.thread.process.upgrade().unwrap().lock().enqueue_ready_thread(self);
        };
    }

    /// Gets a reference to the register values of this thread. These values are only updated when a thread stops running. If this thread is
    /// currently in the running state, then these values will be stale.
    pub fn regs(&self) -> &SavedRegisters {
        &self.guard.regs
    }

    /// Gets a mutable reference to the register values of this thread. These values are updated whenever a thread stops running. If this
    /// thread is currently in the running state, then these values will be stale and any updates to these values will not take effect.
    ///
    /// # Safety
    ///
    /// Manually modifying the registers of a thread can cause numerous safety issues and must be done with extreme caution.
    ///
    /// For threads currently executing kernel code, this method should only be used internally by the scheduler to save registers during
    /// context switches and in very specific debugging scenarios where kernel integrity is not a concern. Modifying register values of
    /// such threads can cause serious security issues.
    ///
    /// For threads currently executing user-mode code, this method can be used to implement debugging and tracing facilities. Care needs to
    /// be taken when doing this to avoid modifying registers in such a way as to allow privilege escalation, e.g. the segment registers on
    /// x86 must never be written with untrusted values.
    pub unsafe fn regs_mut(&mut self) -> &mut SavedRegisters {
        &mut self.guard.regs
    }
}
