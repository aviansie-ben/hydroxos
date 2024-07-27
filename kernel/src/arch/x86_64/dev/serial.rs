use alloc::boxed::Box;

use dyn_dyn::dyn_dyn_impl;

use crate::io::dev::{self, Device, DeviceNode, DeviceRef};
use crate::io::tty::Tty;
use crate::sync::{Future, UninterruptibleSpinlock};

#[derive(Debug)]
pub struct SerialPort {
    port: UninterruptibleSpinlock<uart_16550::SerialPort>
}

#[dyn_dyn_impl(Tty)]
impl Device for SerialPort {}

impl Tty for SerialPort {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>> {
        let mut port = self.port.lock();

        for &b in bytes.as_ref().unwrap() {
            port.send_raw(b);
        }

        Future::done(Ok(()))
    }

    unsafe fn flush(&self) -> Future<Result<(), ()>> {
        Future::done(Ok(()))
    }

    unsafe fn read(&self, bytes: *mut [u8]) -> Future<Result<usize, ()>> {
        let mut port = self.port.lock();

        for i in 0..bytes.len() {
            let mut b = port.receive();

            // TODO This should be controllable, or binary data would be a real problem
            if b == b'\r' {
                b = b'\n';
            }

            *bytes.get_unchecked_mut(i) = b;
        }

        Future::done(Ok(bytes.len()))
    }
}

pub unsafe fn init() -> DeviceRef<SerialPort> {
    let mut port = uart_16550::SerialPort::new(0x3f8);
    port.init();

    dev::device_root()
        .dev()
        .add_device(DeviceNode::new(Box::from("serial0"), SerialPort {
            port: UninterruptibleSpinlock::new(port)
        }))
}
