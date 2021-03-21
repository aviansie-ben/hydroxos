use core::fmt;

use crate::future::Future;

pub trait Tty {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>>;
    unsafe fn flush(&self) -> Future<Result<(), ()>>;

    unsafe fn read(&self, bytes: *mut [u8]) -> Future<Result<usize, ()>>;
}

pub struct TtyWriter<'a, T: Tty>(&'a T);

impl <'a, T: Tty> TtyWriter<'a, T> {
    pub fn new(val: &'a T) -> Self {
        TtyWriter(val)
    }
}

impl <T: Tty> fmt::Write for TtyWriter<'_, T> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe {
            match self.0.write(s.as_bytes() as *const [u8]).unwrap_blocking() {
                Ok(()) => Ok(()),
                Err(_) => Err(fmt::Error)
            }
        }
    }
}
