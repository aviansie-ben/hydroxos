use alloc::collections::VecDeque;
use core::fmt;

use crate::{
    sync::{future::FutureWriter, Future},
    util::ArrayDeque,
};

pub trait Tty: Send + Sync {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>>;
    unsafe fn flush(&self) -> Future<Result<(), ()>>;

    unsafe fn read(&self, bytes: *mut [u8]) -> Future<Result<usize, ()>>;

    fn size(&self) -> Result<(usize, usize), ()> {
        Err(())
    }
}

pub trait TtyExt: Tty {
    fn write_blocking(&self, bytes: &[u8]) -> Result<(), ()> {
        unsafe { self.write(bytes).unwrap_blocking() }
    }

    fn read_blocking(&self, bytes: &mut [u8]) -> Result<usize, ()> {
        unsafe { self.read(bytes).unwrap_blocking() }
    }
}

impl<T: Tty + ?Sized> TtyExt for T {}

pub struct TtyWriter<'a, T: Tty + ?Sized>(&'a T);

impl<'a, T: Tty + ?Sized> TtyWriter<'a, T> {
    pub fn new(val: &'a T) -> Self {
        TtyWriter(val)
    }
}

impl<T: Tty + ?Sized> fmt::Write for TtyWriter<'_, T> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe {
            match self.0.write(s.as_bytes() as *const [u8]).unwrap_blocking() {
                Ok(()) => Ok(()),
                Err(_) => Err(fmt::Error),
            }
        }
    }
}

pub struct TtyCharReader<'a, T: Tty + ?Sized>(&'a T);

impl<'a, T: Tty + ?Sized> TtyCharReader<'a, T> {
    pub fn new(val: &'a T) -> Self {
        TtyCharReader(val)
    }

    pub fn next_char(&mut self) -> Result<char, ()> {
        fn is_valid_start_byte(b: u8) -> bool {
            (b & 0xc0) != 0x80
        }

        let mut buf = [0_u8; 4];
        let mut pos = 0;

        Ok(loop {
            self.0.read_blocking(&mut buf[pos..pos + 1])?;
            if pos == 0 && !is_valid_start_byte(buf[0]) {
                continue;
            } else if buf[pos] >= 0xf8 {
                break '\u{fffd}';
            }

            pos += 1;

            if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
                break s.chars().next().unwrap();
            }
        })
    }
}

#[derive(Debug)]
struct TtyReadRequest {
    future: FutureWriter<Result<usize, ()>>,
    buf: *mut [u8],
    pos: usize,
}

impl TtyReadRequest {
    fn complete(self) {
        assert!(self.pos == self.buf.len());
        self.cancel();
    }

    fn cancel(self) {
        self.future.finish(if self.pos > 0 { Ok(self.pos) } else { Err(()) })
    }
}

#[derive(Debug)]
pub struct TtyReadQueue<const N: usize> {
    buf: ArrayDeque<u8, N>,
    requests: VecDeque<TtyReadRequest>,
}

impl<const N: usize> TtyReadQueue<N> {
    pub fn new() -> Self {
        Self {
            buf: ArrayDeque::new(),
            requests: VecDeque::new(),
        }
    }

    pub fn has_room(&self, size: usize) -> bool {
        size < N && N - self.buf.len() >= size
    }

    pub fn push_bytes(&mut self, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            if let Some(request) = self.requests.front_mut() {
                unsafe {
                    let buf_len = (*request.buf).len();

                    let copy_begin = request.pos;
                    let copy_end = copy_begin.saturating_add(data.len()).min(buf_len);
                    let copy_len = copy_end - request.pos;

                    (*request.buf)[copy_begin..copy_end].copy_from_slice(&data[..copy_len]);
                    data = &data[copy_len..];

                    request.pos = copy_end;
                    if copy_end == buf_len {
                        self.requests.pop_front().unwrap().complete();
                    }
                }
            } else {
                for &byte in data {
                    if self.buf.push_back(byte).is_err() {
                        return false;
                    }
                }
                data = &data[data.len()..];
            }
        }

        true
    }

    pub unsafe fn read(&mut self, bytes: *mut [u8]) -> Future<Result<usize, ()>> {
        let mut pos = 0;

        while pos < (bytes.len()) {
            if let Some(b) = self.buf.pop_front() {
                (*bytes)[pos] = b;
                pos += 1;
            } else {
                break;
            }
        }

        if pos < bytes.len() {
            let (future, future_writer) = Future::new();

            self.requests.push_back(TtyReadRequest {
                future: future_writer,
                buf: bytes,
                pos,
            });

            future
        } else {
            Future::done(Ok(pos))
        }
    }
}
