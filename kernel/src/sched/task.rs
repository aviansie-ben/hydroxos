pub mod x86_64 {
    use crate::util::SharedUnsafeCell;

    pub const XSAVE_MAX_EXTENDED_SIZE: usize = 0;

    static XSAVE_ENABLED: SharedUnsafeCell<bool> = SharedUnsafeCell::new(false);
    pub fn xsave_enabled() -> bool {
        unsafe { *XSAVE_ENABLED.get() }
    }

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

    #[derive(Clone, Debug)]
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
        pub mm: [[u8; 16]; 8],
        pub xmm: [[u8; 16]; 16],
        pub reserved_1: [u8; 96],
        pub xstate_bv: [u8; 8],
        pub xcomp_bv: [u8; 8],
        pub reserved_2: [u8; 48],
        pub xsave_extended: [u8; XSAVE_MAX_EXTENDED_SIZE]
    }

    impl SavedExtendedRegisters {
        pub fn new() -> SavedExtendedRegisters {
            SavedExtendedRegisters {
                fcw: 0x037F,
                fsw: 0,
                ftw: 0,
                reserved_0: 0,
                fop: 0,
                fip: 0,
                fdp: 0,
                mxcsr: 0x1F80,
                mxcsr_mask: 0xFFFF,
                mm: [[0; 16]; 8],
                xmm: [[0; 16]; 16],
                reserved_1: [0; 96],
                xstate_bv: [0; 8],
                xcomp_bv: [0; 8],
                reserved_2: [0; 48],
                xsave_extended: [0; XSAVE_MAX_EXTENDED_SIZE]
            }
        }

        pub fn save(&mut self) {
            unsafe {
                if xsave_enabled() {
                    asm!("xsave [{}]", in(reg) self, in("eax") -1, in("edx") -1);
                } else {
                    asm!("fxsave [{}]", in(reg) self);
                };
            };
        }

        pub fn restore(&self) {
            unsafe {
                if xsave_enabled() {
                    asm!("xrstor [{}]", in(reg) self, in("eax") -1, in("edx") -1);
                } else {
                    asm!("fxrstor [{}]", in(reg) self);
                };
            };
        }
    }

    pub struct SavedRegisters {
        pub basic: SavedBasicRegisters,
        pub ext: SavedExtendedRegisters
    }

    pub unsafe fn init_xsave() {
        use x86_64::registers::control::{Cr4, Cr4Flags};
        use crate::x86_64::cpuid::{self, CpuFeature};

        if cpuid::get_minimum_features().supports(CpuFeature::XSAVE) {
            Cr4::write(Cr4::read() | Cr4Flags::OSXSAVE);

            let xsave_feature_set_lo: u32 = 0x00000003; // x87 and SSE
            let xsave_feature_set_hi: u32 = 0x00000000;

            asm!(
                "mov ecx, 0",
                "xsetbv",
                in("eax") xsave_feature_set_lo,
                in("edx") xsave_feature_set_hi
            );

            *XSAVE_ENABLED.get() = true;
        };
    }

    #[cfg(test)]
    mod test {
        use super::SavedExtendedRegisters;

        pub const XMM_ZERO: [u8; 16] = [0; 16];
        pub const XMM0_VAL: [u8; 16] = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF,
            0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10
        ];
        pub const XMM14_VAL: [u8; 16] = [
            0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF,
            0xCA, 0xFE, 0xDE, 0xAD, 0xCA, 0xFE, 0xD0, 0x0D
        ];

        #[test_case]
        fn test_save_xmm() {
            let mut state = SavedExtendedRegisters::new();

            unsafe {
                asm!("movdqu xmm0, [{}]", in(reg) &XMM0_VAL);
                asm!("movdqu xmm14, [{}]", in(reg) &XMM14_VAL);
            };

            state.save();
            assert_eq!(XMM0_VAL, state.xmm[0]);
            assert_eq!(XMM14_VAL, state.xmm[14]);
        }

        #[test_case]
        fn test_restore_xmm() {
            let mut state = SavedExtendedRegisters::new();

            unsafe {
                asm!(
                    "movdqu xmm0, [{}]",
                    "movdqu xmm14, xmm0",
                    in(reg) &XMM_ZERO
                );
            };

            state.save();
            state.xmm[0] = XMM0_VAL;
            state.xmm[14] = XMM14_VAL;
            state.restore();

            let mut xmm0 = XMM_ZERO;
            let mut xmm14 = XMM_ZERO;

            unsafe {
                asm!("movdqu [{}], xmm0", in(reg) &mut xmm0);
                asm!("movdqu [{}], xmm14", in(reg) &mut xmm14);
            };

            assert_eq!(XMM0_VAL, xmm0);
            assert_eq!(XMM14_VAL, xmm14);
        }
    }
}
