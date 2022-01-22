use core::arch::asm;
use core::mem;

use crate::util::SharedUnsafeCell;

pub const XSAVE_AVX_SIZE: usize = 256;
pub const XSAVE_MAX_EXTENDED_SIZE: usize = XSAVE_AVX_SIZE + 1024;

static XSAVE_ENABLED: SharedUnsafeCell<bool> = SharedUnsafeCell::new(false);
static XSAVE_AVX_OFFSET: SharedUnsafeCell<usize> = SharedUnsafeCell::new(!0);

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
    pub gs: u16,
    pub fsbase: u64,
    pub gsbase: u64
}

impl SavedBasicRegisters {
    pub fn new() -> SavedBasicRegisters {
        SavedBasicRegisters {
            rip: 0,
            rflags: 0,
            gprs: [0; 16],
            cs: 0,
            ss: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
            fsbase: 0,
            gsbase: 0
        }
    }

    pub fn gpr(&self, reg: GeneralRegister) -> u64 {
        self.gprs[reg as usize]
    }

    pub fn set_gpr(&mut self, reg: GeneralRegister, val: u64) {
        self.gprs[reg as usize] = val;
    }

    pub fn new_kernel_thread(f: extern "C" fn(*mut u8) -> !, arg: *mut u8, stack: *mut u8) -> SavedBasicRegisters {
        let mut regs = SavedBasicRegisters::new();

        regs.rip = f as u64;
        regs.set_gpr(GeneralRegister::Rdi, arg as u64);
        regs.set_gpr(GeneralRegister::Rsp, stack as u64);

        regs.cs = 0x08;
        regs.ss = 0x10;
        regs.ds = 0x10;
        regs.es = 0x10;
        regs.fs = 0x10;
        regs.gs = 0x10;

        regs
    }

    pub fn new_user_thread(f: u64, arg: u64, stack: u64) -> SavedBasicRegisters {
        let mut regs = SavedBasicRegisters::new();

        regs.rip = f;
        regs.rflags |= 1 << 9; // IF
        regs.rflags |= 0x3 << 12; // IOPL
        regs.set_gpr(GeneralRegister::Rdi, arg);
        regs.set_gpr(GeneralRegister::Rsp, stack);

        regs.cs = 0x18;
        regs.ss = 0x20;
        regs.ds = 0x20;
        regs.es = 0x20;
        regs.fs = 0x20;
        regs.gs = 0x20;

        regs
    }
}

fn to_ymm_val(lo: &[u8; 16], hi: &[u8; 16]) -> [u8; 32] {
    let mut result = [0; 32];

    for i in 0..16 {
        result[i] = lo[i];
    }

    for i in 0..16 {
        result[i + 16] = hi[i];
    }

    result
}

fn from_ymm_val(val: &[u8; 32]) -> ([u8; 16], [u8; 16]) {
    let mut lo = [0; 16];
    let mut hi = [0; 16];

    for i in 0..16 {
        lo[i] = val[i];
    }

    for i in 0..16 {
        hi[i] = val[i + 16];
    }

    (lo, hi)
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct XSaveAvx {
    pub ymm_h: [[u8; 16]; 16]
}

impl XSaveAvx {
    fn new() -> XSaveAvx {
        XSaveAvx { ymm_h: [[0; 16]; 16] }
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct XSaveExtendedArea([u8; XSAVE_MAX_EXTENDED_SIZE]);

impl XSaveExtendedArea {
    fn new() -> XSaveExtendedArea {
        XSaveExtendedArea([0; XSAVE_MAX_EXTENDED_SIZE])
    }

    fn ext_avx(&self) -> Option<&XSaveAvx> {
        unsafe {
            if *XSAVE_AVX_OFFSET.get() != !0 {
                Some(mem::transmute(&self.0[*XSAVE_AVX_OFFSET.get()]))
            } else {
                None
            }
        }
    }

    fn ext_avx_mut(&mut self) -> Option<&mut XSaveAvx> {
        unsafe {
            if *XSAVE_AVX_OFFSET.get() != !0 {
                Some(mem::transmute(&mut self.0[*XSAVE_AVX_OFFSET.get()]))
            } else {
                None
            }
        }
    }
}

#[derive(Clone, Debug)]
#[repr(C, align(64))]
pub struct SavedExtendedRegisters {
    fcw: u16,
    fsw: u16,
    ftw: u8,
    reserved_0: u8,
    fop: u16,
    fip: u64,
    fdp: u64,
    mxcsr: u32,
    mxcsr_mask: u32,
    st: [([u8; 10], [u8; 6]); 8],
    xmm: [[u8; 16]; 16],
    reserved_1: [u8; 96],
    xstate_bv: [u8; 8],
    xcomp_bv: [u8; 8],
    reserved_2: [u8; 48],
    xsave_extended: XSaveExtendedArea
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
            st: [([0; 10], [0; 6]); 8],
            xmm: [[0; 16]; 16],
            reserved_1: [0; 96],
            xstate_bv: [0; 8],
            xcomp_bv: [0; 8],
            reserved_2: [0; 48],
            xsave_extended: XSaveExtendedArea::new()
        }
    }

    pub fn save(&mut self) {
        unsafe {
            if xsave_enabled() {
                asm!("xsave [{}]", in(reg) self, in("eax") -1, in("edx") -1);
            } else {
                self.xstate_bv[0] = 0x3;
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

    pub fn st(&self, idx: usize) -> [u8; 10] {
        if (self.xstate_bv[0] & 0x01) != 0 {
            self.st[idx].0
        } else {
            [0; 10]
        }
    }

    pub fn set_st(&mut self, idx: usize, val: [u8; 10]) {
        if (self.xstate_bv[0] & 0x01) == 0 {
            self.st = [([0; 10], [0; 6]); 8];
            self.xstate_bv[0] |= 0x01;
        };

        self.st[idx].0 = val;
    }

    pub fn xmm(&self, idx: usize) -> [u8; 16] {
        if (self.xstate_bv[0] & 0x02) != 0 {
            self.xmm[idx]
        } else {
            [0; 16]
        }
    }

    pub fn set_xmm(&mut self, idx: usize, val: [u8; 16]) {
        if (self.xstate_bv[0] & 0x02) == 0 {
            self.xmm = [[0; 16]; 16];
            self.xstate_bv[0] |= 0x02;
        };
        self.xmm[idx] = val;
    }

    pub fn ymm(&self, idx: usize) -> Result<[u8; 32], ()> {
        if let Some(ext_avx) = self.xsave_extended.ext_avx() {
            if (self.xstate_bv[0] & 0x02) == 0 {
                Ok([0; 32])
            } else if (self.xstate_bv[0] & 0x04) == 0 {
                Ok(to_ymm_val(&self.xmm[idx], &[0; 16]))
            } else {
                Ok(to_ymm_val(&self.xmm[idx], &ext_avx.ymm_h[idx]))
            }
        } else {
            Err(())
        }
    }

    pub fn set_ymm(&mut self, idx: usize, val: [u8; 32]) -> Result<(), ()> {
        if (self.xstate_bv[0] & 0x02) == 0 {
            self.xmm = [[0; 16]; 16];
            self.xstate_bv[0] |= 0x02;
        };

        if (self.xstate_bv[0] & 0x04) == 0 {
            if let Some(ext_avx) = self.xsave_extended.ext_avx_mut() {
                *ext_avx = XSaveAvx::new();
                self.xstate_bv[0] |= 0x04;
            };
        };

        if let Some(ext_avx) = self.xsave_extended.ext_avx_mut() {
            let (lo, hi) = from_ymm_val(&val);

            ext_avx.ymm_h[idx] = hi;
            self.xmm[idx] = lo;
            Ok(())
        } else {
            Err(())
        }
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

pub unsafe fn init_xsave() {
    use x86_64::registers::control::{Cr4, Cr4Flags};

    use crate::x86_64::cpuid::{self, CpuFeature};

    if cpuid::get_minimum_features().supports(CpuFeature::XSAVE) {
        Cr4::write(Cr4::read() | Cr4Flags::OSXSAVE);

        let mut current_offset = 0;
        let mut xsave_feature_set_lo: u32 = 0x00000003; // x87 and SSE
        let xsave_feature_set_hi: u32 = 0x00000000;

        if cpuid::get_minimum_features().supports(CpuFeature::AVX) {
            xsave_feature_set_lo |= 0x00000004;
            *XSAVE_AVX_OFFSET.get() = current_offset;
            current_offset += XSAVE_AVX_SIZE;
        };

        assert!(current_offset <= XSAVE_MAX_EXTENDED_SIZE);

        asm!(
            "mov ecx, 0",
            "xsetbv",
            in("eax") xsave_feature_set_lo,
            in("edx") xsave_feature_set_hi
        );

        *XSAVE_ENABLED.get() = true;
    } else {
        assert!(!cpuid::get_minimum_features().supports(CpuFeature::AVX));
    };
}

#[cfg(test)]
mod test {
    use core::arch::asm;

    use super::super::cpuid::{self, CpuFeature};
    use super::SavedExtendedRegisters;
    use crate::test_util::skip;

    pub const ST_ZERO: [u8; 10] = [0; 10];
    pub const ST_TEST: [u8; 10] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A];

    pub const XMM_ZERO: [u8; 16] = [0; 16];
    pub const XMM0_VAL: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, //
        0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10
    ];
    pub const XMM14_VAL: [u8; 16] = [
        0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF, //
        0xCA, 0xFE, 0xDE, 0xAD, 0xCA, 0xFE, 0xD0, 0x0D
    ];

    pub const YMM_ZERO: [u8; 32] = [0; 32];
    pub const YMM0_VAL: [u8; 32] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, //
        0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, //
        0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, //
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF
    ];
    pub const YMM14_VAL: [u8; 32] = [
        0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF, //
        0xCA, 0xFE, 0xDE, 0xAD, 0xCA, 0xFE, 0xD0, 0x0D, //
        0xCA, 0xFE, 0xD0, 0x0D, 0xCA, 0xFE, 0xDE, 0xAD, //
        0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF, 0xBE, 0xEF
    ];

    #[test_case]
    fn test_save_st() {
        let mut state = SavedExtendedRegisters::new();

        unsafe {
            asm!("fld tbyte ptr [{}]", in(reg) &ST_TEST);
        };

        state.save();

        unsafe {
            asm!("fincstp");
            asm!("ffree st(0)");
        };

        assert_eq!(ST_TEST, state.st(0));
    }

    #[test_case]
    fn test_restore_st() {
        let mut state = SavedExtendedRegisters::new();

        unsafe {
            asm!("fld tbyte ptr [{}]", in(reg) &ST_ZERO);
        };

        state.save();
        state.set_st(0, ST_TEST);
        state.restore();

        let mut st0 = [0; 10];

        unsafe {
            asm!("fstp tbyte ptr [{}]", in(reg) &mut st0);
        };

        assert_eq!(ST_TEST, st0);
    }

    #[test_case]
    fn test_save_xmm() {
        let mut state = SavedExtendedRegisters::new();

        unsafe {
            asm!("movdqu xmm0, [{}]", in(reg) &XMM0_VAL);
            asm!("movdqu xmm14, [{}]", in(reg) &XMM14_VAL);
        };

        state.save();
        assert_eq!(XMM0_VAL, state.xmm(0));
        assert_eq!(XMM14_VAL, state.xmm(14));
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
        state.set_xmm(0, XMM0_VAL);
        state.set_xmm(14, XMM14_VAL);
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

    #[test_case]
    fn test_save_ymm() {
        if !cpuid::get_minimum_features().supports(CpuFeature::AVX) {
            skip("avx not supported");
            return;
        };

        let mut state = SavedExtendedRegisters::new();

        unsafe {
            asm!("vmovdqu ymm0, [{}]", in(reg) &YMM0_VAL);
            asm!("vmovdqu ymm14, [{}]", in(reg) &YMM14_VAL);
        };

        state.save();

        assert_eq!(YMM0_VAL, state.ymm(0).unwrap());
        assert_eq!(YMM14_VAL, state.ymm(14).unwrap());
    }

    #[test_case]
    fn test_restore_ymm() {
        if !cpuid::get_minimum_features().supports(CpuFeature::AVX) {
            skip("avx not supported");
            return;
        };

        let mut state = SavedExtendedRegisters::new();

        unsafe {
            asm!(
                "vmovdqu ymm0, [{}]",
                "vmovdqu ymm14, ymm0",
                in(reg) &YMM_ZERO
            );
        };

        state.save();
        state.set_ymm(0, YMM0_VAL).unwrap();
        state.set_ymm(14, YMM14_VAL).unwrap();
        state.restore();

        assert_eq!(YMM0_VAL, state.ymm(0).unwrap());

        let mut ymm0 = YMM_ZERO;
        let mut ymm14 = YMM_ZERO;

        unsafe {
            asm!("vmovdqu [{}], ymm0", in(reg) &mut ymm0);
            asm!("vmovdqu [{}], ymm14", in(reg) &mut ymm14);
        };

        assert_eq!(YMM0_VAL, ymm0);
        assert_eq!(YMM14_VAL, ymm14);
    }
}
