use crate::util::SharedUnsafeCell;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuFeature {
    feature_vec_idx: u32,
    feature_vec_bit: u32,
    name: &'static str
}

impl CpuFeature {
    const FEATURE_VEC_IDX_01_ECX: u32 = 0;
    const FEATURE_VEC_IDX_01_EDX: u32 = 1;
    const FEATURE_VEC_IDX_MAX: u32 = 1;

    pub const XSAVE: CpuFeature = CpuFeature {
        feature_vec_idx: CpuFeature::FEATURE_VEC_IDX_01_ECX,
        feature_vec_bit: 1 << 26,
        name: "xsave"
    };
}

pub struct CpuFeatureSet([u32; CpuFeatureSet::NUM_FEATURE_VECS]);

impl CpuFeatureSet {
    const NUM_FEATURE_VECS: usize = (CpuFeature::FEATURE_VEC_IDX_MAX + 1) as usize;

    pub const fn empty() -> CpuFeatureSet {
        CpuFeatureSet([0; CpuFeatureSet::NUM_FEATURE_VECS])
    }

    pub fn detect() -> CpuFeatureSet {
        let mut features = [0; CpuFeatureSet::NUM_FEATURE_VECS];

        unsafe {
            asm!(
                "mov eax, 1",
                "cpuid",
                out("eax") _,
                out("ebx") _,
                out("ecx") features[CpuFeature::FEATURE_VEC_IDX_01_ECX as usize],
                out("edx") features[CpuFeature::FEATURE_VEC_IDX_01_EDX as usize]
            );
        };

        CpuFeatureSet(features)
    }

    pub fn supports(&self, feature: CpuFeature) -> bool {
        (self.0[feature.feature_vec_idx as usize] & feature.feature_vec_bit) != 0
    }
}

static MIN_FEATURES: SharedUnsafeCell<CpuFeatureSet> = SharedUnsafeCell::new(CpuFeatureSet::empty());

pub unsafe fn init_bsp() {
    *MIN_FEATURES.get() = CpuFeatureSet::detect();
}

pub fn get_minimum_features() -> &'static CpuFeatureSet {
    unsafe { &*MIN_FEATURES.get() }
}
