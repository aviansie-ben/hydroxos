use crate::arch::regs::SavedBasicRegisters;

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct InterruptFrame {}

impl InterruptFrame {
    pub fn save(&self, saved: &mut SavedBasicRegisters) {
        unimplemented!()
    }

    pub fn restore(&mut self, saved: &SavedBasicRegisters) {
        unimplemented!()
    }

    pub fn set_to_idle(&mut self) {
        unimplemented!()
    }

    pub fn setup_kernel_mode_thread_locals(&mut self) {
        unimplemented!()
    }
}

pub fn are_enabled() -> bool {
    unimplemented!()
}

pub fn enable() {
    unimplemented!()
}

pub fn disable() {
    unimplemented!()
}
