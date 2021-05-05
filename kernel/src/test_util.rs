use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;
use uart_16550::SerialPort;

static TEST_SERIAL: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3f8) });
static IS_SKIPPED: AtomicBool = AtomicBool::new(false);
static IS_TESTING: AtomicBool = AtomicBool::new(false);

pub trait Test {
    fn run(&self);
}

impl <T: Fn ()> Test for T {
    fn run(&self) {
        write!(TEST_SERIAL.lock(), "test {} ... ", core::any::type_name::<T>()).unwrap();
        IS_SKIPPED.store(false, Ordering::Relaxed);
        IS_TESTING.store(true, Ordering::Relaxed);
        self();
        IS_TESTING.store(false, Ordering::Relaxed);

        if !IS_SKIPPED.load(Ordering::Relaxed) {
            writeln!(TEST_SERIAL.lock(), "\x1b[32mok\x1b[0m").unwrap();
        };
    }
}

pub fn run_tests(tests: &[&dyn Test]) -> ! {
    TEST_SERIAL.lock().init();
    writeln!(TEST_SERIAL.lock(), "Running {} tests...", tests.len()).unwrap();
    for test in tests {
        test.run();
    };

    exit(0);
}

pub fn skip(reason: &str) {
    writeln!(TEST_SERIAL.lock(), "skipped ({})", reason).unwrap();
    IS_SKIPPED.store(true, Ordering::Relaxed);
}

pub fn exit(code: u32) -> ! {
    use crate::x86_64::dev::qemu_dbg_exit::QemuExitDevice;

    unsafe { QemuExitDevice::new(0xf4).exit(code) }
}

pub fn handle_test_panic(info: &PanicInfo) -> ! {
    let mut serial_lock = TEST_SERIAL.lock();

    if IS_TESTING.load(Ordering::Relaxed) {
        let _ = writeln!(serial_lock, "\x1b[31mFAILED\x1b[0m");
    };

    let _ = writeln!(serial_lock, "{}", info);
    exit(1);
}
