use core::fmt::Write;
use core::panic::PanicInfo;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use dyn_dyn::dyn_dyn_impl;
use uart_16550::SerialPort;

use crate::io::dev::{device_root, Device, DeviceNode};
use crate::io::tty::Tty;
use crate::sched::is_handling_interrupt;
use crate::sched::task::{Process, Thread};
use crate::sync::uninterruptible::InterruptDisabler;
use crate::sync::{Future, UninterruptibleSpinlock};

pub const TEST_THREAD_STACK_SIZE: usize = 16 * 4096;

pub static TEST_SERIAL: UninterruptibleSpinlock<SerialPort> = UninterruptibleSpinlock::new(unsafe { SerialPort::new(0x3f8) });

static TEST_LOG_PRINTED_NEWLINE: AtomicBool = AtomicBool::new(false);
static IS_SKIPPED: AtomicBool = AtomicBool::new(false);
static IS_TESTING: AtomicBool = AtomicBool::new(false);
static TEST_FAILED: AtomicBool = AtomicBool::new(false);
static TEST_IDX: AtomicUsize = AtomicUsize::new(0);
static TEST_THREAD: AtomicPtr<Thread> = AtomicPtr::new(ptr::null_mut());

#[derive(Debug)]
pub struct TestLogTty;

impl Tty for TestLogTty {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>> {
        let mut serial_lock = TEST_SERIAL.lock();

        if IS_TESTING.load(Ordering::Relaxed) && !TEST_LOG_PRINTED_NEWLINE.swap(true, Ordering::Relaxed) {
            serial_lock.send_raw(b'\n');
        }

        for &b in bytes.as_ref().unwrap() {
            serial_lock.send_raw(b);
        }

        Future::done(Ok(()))
    }

    unsafe fn flush(&self) -> Future<Result<(), ()>> {
        Future::done(Ok(()))
    }

    unsafe fn read(&self, _bytes: *mut [u8]) -> Future<Result<usize, ()>> {
        Future::done(Err(()))
    }
}

#[dyn_dyn_impl(Tty)]
impl Device for TestLogTty {}

pub trait Test: Sync {
    fn run(&self);
}

impl<T: Fn() + Sync> Test for T {
    fn run(&self) {
        write!(TEST_SERIAL.lock(), "test {} ... ", core::any::type_name::<T>()).unwrap();
        TEST_LOG_PRINTED_NEWLINE.store(false, Ordering::Relaxed);
        IS_SKIPPED.store(false, Ordering::Relaxed);
        IS_TESTING.store(true, Ordering::Relaxed);
        self();
        IS_TESTING.store(false, Ordering::Relaxed);

        if !IS_SKIPPED.load(Ordering::Relaxed) {
            writeln!(TEST_SERIAL.lock(), "\x1b[32mok\x1b[0m").unwrap();
        };
    }
}

pub fn init_test_log() {
    use alloc::boxed::Box;

    use crate::log;

    TEST_SERIAL.lock().init();
    log::init(device_root().dev().add_device(DeviceNode::new(Box::from("testlog"), TestLogTty)));
}

pub fn run_tests(tests: &'static [&dyn Test]) -> ! {
    writeln!(TEST_SERIAL.lock(), "Running {} tests...", tests.len()).unwrap();

    loop {
        let tests = &tests[TEST_IDX.load(Ordering::Relaxed)..];

        if tests.is_empty() {
            break;
        }

        let test_thread = Process::kernel()
            .lock()
            .create_kernel_thread(move || run_tests_thread(tests), TEST_THREAD_STACK_SIZE);
        let test_thread_complete = test_thread.lock().join();

        TEST_THREAD.store(&*test_thread as *const _ as *mut _, Ordering::Relaxed);

        test_thread.lock().wake();
        test_thread_complete.unwrap_blocking();
    }

    exit(if TEST_FAILED.load(Ordering::Relaxed) { 1 } else { 0 });
}

pub fn run_tests_thread(tests: &[&dyn Test]) {
    x86_64::instructions::interrupts::enable();

    for test in tests {
        TEST_IDX.fetch_add(1, Ordering::Relaxed);
        test.run();
    }
}

pub fn skip(reason: &str) {
    writeln!(TEST_SERIAL.lock(), "skipped ({})", reason).unwrap();
    IS_SKIPPED.store(true, Ordering::Relaxed);
}

#[cfg(not(feature = "check_arch_api"))]
pub fn exit(code: u32) -> ! {
    use crate::arch::x86_64::dev::qemu_dbg_exit::QemuExitDevice;

    unsafe { QemuExitDevice::new(0xf4).exit(code) }
}

#[cfg(feature = "check_arch_api")]
pub fn exit(_code: u32) -> ! {
    crate::arch::halt();
}

pub fn handle_test_panic(info: &PanicInfo) -> ! {
    let mut serial_lock = TEST_SERIAL.lock();
    let is_testing = IS_TESTING.swap(false, Ordering::Relaxed);

    if is_testing {
        let _ = writeln!(serial_lock, "\x1b[31mFAILED\x1b[0m");
    }

    let _ = writeln!(serial_lock, "{}", info);

    if is_testing {
        if is_handling_interrupt() {
            let _ = writeln!(serial_lock, "Unable to continue testing, since panic occurred during an interrupt");
            exit(1);
        } else if !ptr::eq(&*Thread::current(), TEST_THREAD.load(Ordering::Relaxed)) {
            let _ = writeln!(
                serial_lock,
                "Unable to continue testing, since panic didn't occur on the test thread"
            );
            exit(1);
        } else if InterruptDisabler::num_held() > 1 {
            let _ = writeln!(serial_lock, "Unable to continue testing due to live InterruptDisabler");
            exit(1);
        } else if !TEST_FAILED.swap(true, Ordering::Relaxed) {
            let _ = writeln!(
                serial_lock,
                "WARNING: Trying to continue testing. System may be unstable after this point."
            );
        }

        drop(serial_lock);
        unsafe {
            Thread::kill_current();
        }
    } else {
        exit(1);
    }
}
