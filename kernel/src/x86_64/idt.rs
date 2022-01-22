use core::arch::asm;
use core::mem;

use x86_64::instructions::tables::lidt;
use x86_64::structures::DescriptorTablePointer;
use x86_64::{PrivilegeLevel, VirtAddr};

use super::regs::{GeneralRegister, SavedBasicRegisters};
use crate::util::SharedUnsafeCell;

macro_rules! handler_with_code {
    ($name:ident, $n:expr) => {
        #[naked]
        extern "C" fn $name() {
            unsafe {
                asm!(
                    "push {}",
                    "jmp {}",
                    const $n,
                    sym crate::x86_64::idt::begin_interrupt_common,
                    options(noreturn)
                );
            };
        }
    }
}

macro_rules! handler_without_code {
    ($name:ident, $n:expr) => {
        #[naked]
        extern "C" fn $name() {
            unsafe {
                asm!(
                    "push 0",
                    "push {}",
                    "jmp {}",
                    const $n,
                    sym crate::x86_64::idt::begin_interrupt_common,
                    options(noreturn)
                );
            }
        }
    }
}

#[naked]
unsafe extern "C" fn begin_interrupt_common() {
    asm!(
        // Save all general-purpose registers and segment selectors that weren't automatically pushed by the CPU when starting the
        // interrupt.
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rbp",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rax, ds",
        "push rax",
        "mov rax, es",
        "push rax",
        "push fs",
        "push gs",
        "mov ecx, 0xc0000100",
        "rdmsr",
        "shl rdx, 32",
        "or rax, rdx",
        "push rax",
        "add ecx, 1",
        "rdmsr",
        "shl rdx, 32",
        "or rax, rdx",
        "push rax",
        // Load the kernel's data segment.
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov fs, ax",
        "mov gs, ax",
        // Set the base pointer to 0 so that stack traces end here.
        "mov rbp, 0",
        // Call the Rust handle_interrupt function with the address of the top of the saved registers on the stack.
        "mov rdi, rsp",
        "call {}",
        // Restore the general-purpose registers and segment selectors that were previously saved. Note that handle_interrupt may have
        // modified these values if a context switch is occurring.
        "pop rax",
        "pop rbx",
        "pop gs",
        "pop fs",
        "mov rdx, rax",
        "shr rdx, 32",
        "mov ecx, 0xc0000101",
        "wrmsr",
        "mov rdx, rbx",
        "mov rax, rbx",
        "shr rdx, 32",
        "sub ecx, 1",
        "wrmsr",
        "pop rax",
        "mov es, ax",
        "pop rax",
        "mov ds, ax",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        // Skip the interrupt number and error code that were previously pushed onto the stack and then return from the interrupt.
        "add rsp, 16",
        "iretq",
        sym handle_interrupt,
        options(noreturn)
    );
}

pub const IRQS_START: u8 = 0x20;
pub const EXT_START: u8 = 0x30;

unsafe extern "C" fn handle_interrupt(frame: &mut InterruptFrame) {
    use crate::sched;

    // TODO Load correct FS_BASE based on processor for SMP
    x86_64::registers::model_specific::Msr::new(0xc0000100).write(*super::KERNEL_FS_BASE.get());
    crate::sched::begin_interrupt();

    let interrupt_num = frame.interrupt_num as u8;

    if interrupt_num >= IRQS_START && interrupt_num < EXT_START {
        sched::begin_interrupt();
    };

    // TODO Dynamically register interrupt handlers
    match interrupt_num {
        0x30 => {
            crate::sched::perform_context_switch_interrupt(Some(core::ptr::read(frame.rax as *const crate::sched::task::ThreadLock)), frame);
        },
        _ => {}
    }

    if interrupt_num < IRQS_START {
        panic!("Unhandled exception {} (error code {})", interrupt_num, frame.error_code);
    } else if interrupt_num < EXT_START {
        super::pic::send_eoi(interrupt_num - IRQS_START);
        sched::end_interrupt();
    };

    crate::sched::end_interrupt();
}

handler_without_code!(begin_isr0, 0);
handler_without_code!(begin_isr1, 1);
handler_without_code!(begin_isr2, 2);
handler_without_code!(begin_isr3, 3);
handler_without_code!(begin_isr4, 4);
handler_without_code!(begin_isr5, 5);
handler_without_code!(begin_isr6, 6);
handler_without_code!(begin_isr7, 7);
handler_with_code!(begin_isr8, 8);
handler_without_code!(begin_isr9, 9);
handler_with_code!(begin_isr10, 10);
handler_with_code!(begin_isr11, 11);
handler_with_code!(begin_isr12, 12);
handler_with_code!(begin_isr13, 13);
handler_with_code!(begin_isr14, 14);
handler_without_code!(begin_isr15, 15);
handler_without_code!(begin_isr16, 16);
handler_with_code!(begin_isr17, 17);
handler_without_code!(begin_isr18, 18);
handler_without_code!(begin_isr19, 19);
handler_without_code!(begin_isr20, 20);
handler_without_code!(begin_isr21, 21);
handler_without_code!(begin_isr22, 22);
handler_without_code!(begin_isr23, 23);
handler_without_code!(begin_isr24, 24);
handler_without_code!(begin_isr25, 25);
handler_without_code!(begin_isr26, 26);
handler_without_code!(begin_isr27, 27);
handler_without_code!(begin_isr28, 28);
handler_without_code!(begin_isr29, 29);
handler_with_code!(begin_isr30, 30);
handler_without_code!(begin_isr31, 31);
handler_without_code!(begin_irq0, 32);
handler_without_code!(begin_irq1, 33);
handler_without_code!(begin_irq2, 34);
handler_without_code!(begin_irq3, 35);
handler_without_code!(begin_irq4, 36);
handler_without_code!(begin_irq5, 37);
handler_without_code!(begin_irq6, 38);
handler_without_code!(begin_irq7, 39);
handler_without_code!(begin_irq8, 40);
handler_without_code!(begin_irq9, 41);
handler_without_code!(begin_irq10, 42);
handler_without_code!(begin_irq11, 43);
handler_without_code!(begin_irq12, 44);
handler_without_code!(begin_irq13, 45);
handler_without_code!(begin_irq14, 46);
handler_without_code!(begin_irq15, 47);

handler_without_code!(begin_int30, 0x30);
handler_without_code!(begin_int80, 0x80);

#[repr(C)]
pub struct InterruptFrame {
    pub gsbase: u64,
    pub fsbase: u64,
    pub gs: u64,
    pub fs: u64,
    pub es: u64,
    pub ds: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub interrupt_num: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64
}

impl InterruptFrame {
    pub fn save(&self, saved: &mut SavedBasicRegisters) {
        saved.rip = self.rip;
        saved.rflags = self.rflags;
        saved.gprs[GeneralRegister::Rax as usize] = self.rax;
        saved.gprs[GeneralRegister::Rbx as usize] = self.rbx;
        saved.gprs[GeneralRegister::Rcx as usize] = self.rcx;
        saved.gprs[GeneralRegister::Rdx as usize] = self.rdx;
        saved.gprs[GeneralRegister::Rbp as usize] = self.rbp;
        saved.gprs[GeneralRegister::Rsp as usize] = self.rsp;
        saved.gprs[GeneralRegister::Rsi as usize] = self.rsi;
        saved.gprs[GeneralRegister::Rdi as usize] = self.rdi;
        saved.gprs[GeneralRegister::R8 as usize] = self.r8;
        saved.gprs[GeneralRegister::R9 as usize] = self.r9;
        saved.gprs[GeneralRegister::R10 as usize] = self.r10;
        saved.gprs[GeneralRegister::R11 as usize] = self.r11;
        saved.gprs[GeneralRegister::R12 as usize] = self.r12;
        saved.gprs[GeneralRegister::R13 as usize] = self.r13;
        saved.gprs[GeneralRegister::R14 as usize] = self.r14;
        saved.gprs[GeneralRegister::R15 as usize] = self.r15;
        saved.cs = self.cs as u16;
        saved.ss = self.ss as u16;
        saved.ds = self.ds as u16;
        saved.es = self.es as u16;
        saved.fs = self.fs as u16;
        saved.gs = self.gs as u16;
        saved.fsbase = self.fsbase;
        saved.gsbase = self.gsbase;
    }

    pub fn restore(&mut self, saved: &SavedBasicRegisters) {
        self.rip = saved.rip;
        self.rflags = saved.rflags;
        self.rax = saved.gprs[GeneralRegister::Rax as usize];
        self.rbx = saved.gprs[GeneralRegister::Rbx as usize];
        self.rcx = saved.gprs[GeneralRegister::Rcx as usize];
        self.rdx = saved.gprs[GeneralRegister::Rdx as usize];
        self.rbp = saved.gprs[GeneralRegister::Rbp as usize];
        self.rsp = saved.gprs[GeneralRegister::Rsp as usize];
        self.rsi = saved.gprs[GeneralRegister::Rsi as usize];
        self.rdi = saved.gprs[GeneralRegister::Rdi as usize];
        self.r8 = saved.gprs[GeneralRegister::R8 as usize];
        self.r9 = saved.gprs[GeneralRegister::R9 as usize];
        self.r10 = saved.gprs[GeneralRegister::R10 as usize];
        self.r11 = saved.gprs[GeneralRegister::R11 as usize];
        self.r12 = saved.gprs[GeneralRegister::R12 as usize];
        self.r13 = saved.gprs[GeneralRegister::R13 as usize];
        self.r14 = saved.gprs[GeneralRegister::R14 as usize];
        self.r15 = saved.gprs[GeneralRegister::R15 as usize];
        self.cs = saved.cs as u64;
        self.ds = saved.ds as u64;
        self.es = saved.es as u64;
        self.ss = saved.ss as u64;
        self.fs = saved.fs as u64;
        self.gs = saved.gs as u64;
        self.fsbase = saved.fsbase;
        self.gsbase = saved.gsbase;
    }
}

#[repr(C)]
struct InterruptTableEntry {
    offset_0: u16,
    segment: u16,
    options: u16,
    offset_1: u16,
    offset_2: u32,
    reserved: u32
}

impl InterruptTableEntry {
    const OPTION_TYPE_MASK: u16 = 0x1f00;
    const OPTION_TYPE_INTERRUPT_GATE: u16 = 0x0e00;
    const OPTION_TYPE_TRAP_GATE: u16 = 0x0f00;

    const OPTION_DPL_OFF: u16 = 13;
    const OPTION_DPL_MASK: u16 = 0x0060;

    const OPTION_IST_OFF: u16 = 0;
    const OPTION_IST_MASK: u16 = 0x0007;

    const OPTION_FLAG_PRESENT: u16 = 0x8000;

    const EMPTY: InterruptTableEntry = InterruptTableEntry {
        offset_0: 0,
        segment: 0,
        options: Self::OPTION_TYPE_INTERRUPT_GATE,
        offset_1: 0,
        offset_2: 0,
        reserved: 0
    };

    fn new(ty: u16, dpl: PrivilegeLevel, ist: u16, f: Option<extern "C" fn()>) -> InterruptTableEntry {
        let mut entry = InterruptTableEntry::EMPTY;

        entry.set_type(ty);
        entry.set_dpl(dpl);
        entry.set_ist(ist);
        entry.set_handler(f);

        entry
    }

    fn set_dpl(&mut self, dpl: PrivilegeLevel) {
        self.options &= !Self::OPTION_DPL_MASK;
        self.options |= (dpl as u16) << Self::OPTION_DPL_OFF;
    }

    fn set_type(&mut self, ty: u16) {
        assert_eq!(ty & !Self::OPTION_TYPE_MASK, 0);

        self.options &= !Self::OPTION_TYPE_MASK;
        self.options |= ty;
    }

    fn set_handler(&mut self, f: Option<extern "C" fn()>) {
        if let Some(f) = f {
            let f = f as u64;

            self.segment = x86_64::instructions::segmentation::cs().0;
            self.offset_0 = f as u16;
            self.offset_1 = (f >> 16) as u16;
            self.offset_2 = (f >> 32) as u32;

            self.options |= Self::OPTION_FLAG_PRESENT;
        } else {
            self.options &= !Self::OPTION_FLAG_PRESENT;
        };
    }

    fn set_ist(&mut self, ist: u16) {
        assert_eq!(ist & !Self::OPTION_IST_MASK, 0);

        self.options &= !Self::OPTION_IST_MASK;
        self.options |= ist << Self::OPTION_IST_OFF;
    }
}

#[repr(C, align(16))]
struct InterruptTable {
    entries: [InterruptTableEntry; InterruptTable::NUM_ENTRIES]
}

impl InterruptTable {
    const NUM_ENTRIES: usize = 256;

    const fn new() -> InterruptTable {
        InterruptTable {
            entries: [InterruptTableEntry::EMPTY; InterruptTable::NUM_ENTRIES]
        }
    }

    fn pointer(&self) -> DescriptorTablePointer {
        DescriptorTablePointer {
            base: VirtAddr::new(self as *const _ as u64),
            limit: (mem::size_of::<Self>() - 1) as u16
        }
    }
}

static IDT: SharedUnsafeCell<InterruptTable> = SharedUnsafeCell::new(InterruptTable::new());

pub unsafe fn init_bsp() {
    let idt = IDT.get().as_mut().unwrap();
    let handlers = [
        begin_isr0,
        begin_isr1,
        begin_isr2,
        begin_isr3,
        begin_isr4,
        begin_isr5,
        begin_isr6,
        begin_isr7,
        begin_isr8,
        begin_isr9,
        begin_isr10,
        begin_isr11,
        begin_isr12,
        begin_isr13,
        begin_isr14,
        begin_isr15,
        begin_isr16,
        begin_isr17,
        begin_isr18,
        begin_isr19,
        begin_isr20,
        begin_isr21,
        begin_isr22,
        begin_isr23,
        begin_isr24,
        begin_isr25,
        begin_isr26,
        begin_isr27,
        begin_isr28,
        begin_isr29,
        begin_isr30,
        begin_isr31,
        begin_irq0,
        begin_irq1,
        begin_irq2,
        begin_irq3,
        begin_irq4,
        begin_irq5,
        begin_irq6,
        begin_irq7,
        begin_irq8,
        begin_irq9,
        begin_irq10,
        begin_irq11,
        begin_irq12,
        begin_irq13,
        begin_irq14,
        begin_irq15
    ];

    for (i, f) in handlers.iter().copied().enumerate() {
        idt.entries[i] = InterruptTableEntry::new(InterruptTableEntry::OPTION_TYPE_INTERRUPT_GATE, PrivilegeLevel::Ring0, 0, Some(f));
    }

    idt.entries[0x30] = InterruptTableEntry::new(
        InterruptTableEntry::OPTION_TYPE_TRAP_GATE,
        PrivilegeLevel::Ring0,
        0,
        Some(begin_int30)
    );
    idt.entries[0x80] = InterruptTableEntry::new(
        InterruptTableEntry::OPTION_TYPE_TRAP_GATE,
        PrivilegeLevel::Ring3,
        0,
        Some(begin_int80)
    );

    lidt(&idt.pointer());
}
