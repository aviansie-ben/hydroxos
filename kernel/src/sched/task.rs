pub mod x86_64 {
    pub const XSAVE_MAX_EXTENDED_SIZE: usize = 0;

    #[repr(usize)]
    pub enum GeneralRegister {
        Rax,
        Rbx,
        Rcx,
        Rdx,
        Rbp,
        Rsp,
        Rsi,
        Rdi,
        R8,
        R9,
        R10,
        R11,
        R12,
        R13,
        R14,
        R15
    }

    pub struct SavedBasicRegisters {
        pub rip: u64,
        pub rflags: u64,
        pub gprs: [u64; 16],
        pub cs: u16,
        pub ss: u16,
        pub ds: u16,
        pub es: u16,
        pub fs: u16,
        pub gs: u16
    }

    #[repr(C, align(64))]
    pub struct SavedExtendedRegisters {
        pub fcw: u16,
        pub fsw: u16,
        pub ftw: u8,
        pub reserved_0: u8,
        pub fop: u16,
        pub fip: u64,
        pub fdp: u64,
        pub mxcsr: u32,
        pub mxcsr_mask: u32,
        pub mm: [[u8; 8]; 8],
        pub xmm: [[u8; 8]; 16],
        pub xstate_bv: [u8; 8],
        pub xcomp_bv: [u8; 8],
        pub reserved_1: [u8; 48],
        pub xsave_extended: [u8; XSAVE_MAX_EXTENDED_SIZE]
    }

    pub struct SavedRegisters {
        pub general: SavedBasicRegisters,
        pub ext: SavedExtendedRegisters
    }
}
