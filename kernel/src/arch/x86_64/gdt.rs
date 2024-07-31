use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};

struct GdtConst {
    gdt: GlobalDescriptorTable,
    kernel_cs: SegmentSelector,
    kernel_ds: SegmentSelector,
    user_cs: SegmentSelector,
    user_ds: SegmentSelector,
}

impl GdtConst {
    const fn new() -> GdtConst {
        let mut gdt = GlobalDescriptorTable::new();

        let kernel_cs = gdt.add_entry(Descriptor::kernel_code_segment());
        let kernel_ds = gdt.add_entry(Descriptor::kernel_data_segment());
        let user_cs = gdt.add_entry(Descriptor::user_code_segment());
        let user_ds = gdt.add_entry(Descriptor::user_data_segment());

        GdtConst {
            gdt,
            kernel_cs,
            kernel_ds,
            user_cs,
            user_ds,
        }
    }
}

static GDT: GlobalDescriptorTable = GdtConst::new().gdt;

pub const KERNEL_CS: SegmentSelector = GdtConst::new().kernel_cs;
pub const KERNEL_DS: SegmentSelector = GdtConst::new().kernel_ds;
pub const USER_CS: SegmentSelector = GdtConst::new().user_cs;
pub const USER_DS: SegmentSelector = GdtConst::new().user_ds;

pub(super) unsafe fn init() {
    GDT.load();
}
