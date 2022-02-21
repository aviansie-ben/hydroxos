#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SavedBasicRegisters {}

impl SavedBasicRegisters {
    pub fn new() -> SavedBasicRegisters {
        unimplemented!()
    }

    pub fn new_kernel_thread(f: extern "C" fn(*mut u8) -> !, arg: *mut u8, stack: *mut u8) -> SavedBasicRegisters {
        unimplemented!()
    }

    pub fn new_user_thread(f: u64, arg: u64, stack: u64) -> SavedBasicRegisters {
        unimplemented!()
    }
}

#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct SavedExtendedRegisters {}

impl SavedExtendedRegisters {
    pub fn new() -> SavedExtendedRegisters {
        unimplemented!()
    }

    pub fn save(&mut self) {
        unimplemented!()
    }

    pub fn restore(&self) {
        unimplemented!()
    }
}

pub struct SavedRegisters {
    pub basic: SavedBasicRegisters,
    pub ext: SavedExtendedRegisters
}

impl SavedRegisters {
    pub fn new() -> SavedRegisters {
        SavedRegisters {
            basic: SavedBasicRegisters::new(),
            ext: SavedExtendedRegisters::new()
        }
    }

    pub fn new_kernel_thread(f: extern "C" fn(*mut u8) -> !, arg: *mut u8, stack: *mut u8) -> SavedRegisters {
        SavedRegisters {
            basic: SavedBasicRegisters::new_kernel_thread(f, arg, stack),
            ext: SavedExtendedRegisters::new()
        }
    }

    pub fn new_user_thread(f: u64, arg: u64, stack: u64) -> SavedRegisters {
        SavedRegisters {
            basic: SavedBasicRegisters::new_user_thread(f, arg, stack),
            ext: SavedExtendedRegisters::new()
        }
    }
}
