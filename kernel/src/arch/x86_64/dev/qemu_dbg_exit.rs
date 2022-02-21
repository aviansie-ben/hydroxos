use x86_64::instructions::port::Port;

pub struct QemuExitDevice(u16);

impl QemuExitDevice {
    pub unsafe fn new(port: u16) -> QemuExitDevice {
        QemuExitDevice(port)
    }

    pub fn exit(&mut self, code: u32) -> ! {
        unsafe { Port::new(self.0).write(code) };
        loop {
            ::x86_64::instructions::hlt();
        }
    }
}
