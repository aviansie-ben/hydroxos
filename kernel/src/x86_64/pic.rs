use x86_64::instructions::port::Port;

const MASTER_PIC_COMMAND_PORT: u16 = 0x20;
const MASTER_PIC_DATA_PORT: u16 = 0x21;
const SLAVE_PIC_COMMAND_PORT: u16 = 0xA0;
const SLAVE_PIC_DATA_PORT: u16 = 0xA1;

pub unsafe fn remap_pic(master_off: u8, slave_off: u8) {
    // All writes to PIC ports need to be followed by a wait for I/O to complete. This is done by writing garbage to port 0x80, which is
    // typically unused.
    let mut io_wait_port: Port<u8> = Port::new(0x80);
    let mut io_wait = || {
        io_wait_port.write(0);
    };

    let mut master_pic_command_port: Port<u8> = Port::new(MASTER_PIC_COMMAND_PORT);
    let mut master_pic_data_port: Port<u8> = Port::new(MASTER_PIC_DATA_PORT);

    let mut slave_pic_command_port: Port<u8> = Port::new(SLAVE_PIC_COMMAND_PORT);
    let mut slave_pic_data_port: Port<u8> = Port::new(SLAVE_PIC_DATA_PORT);

    // Save the mask registers for the PICs. These are cleared during initialization and must be restored to their original values
    // afterwards.
    let master_mask = master_pic_data_port.read();
    let slave_mask = slave_pic_data_port.read();

    // Send ICW1 to reset the PICs. Bit 0x10 signifies that this is an ICW1 and that the PIC should reset and bit 0x01 signifies that ICW4
    // will be sent later.
    master_pic_command_port.write(0x11);
    slave_pic_command_port.write(0x11);
    io_wait();

    // Send ICW2 to tell the PICs which IDT offsets should be used.
    master_pic_data_port.write(master_off);
    slave_pic_data_port.write(slave_off);
    io_wait();

    // Send ICW3 to tell the PICs about each other and configure IRQ chaining via the IRQ2 line.
    master_pic_data_port.write(0x04);
    slave_pic_data_port.write(0x02);
    io_wait();

    // Send ICW4 to tell the PICs which mode to operate in. In this case, 8086 mode is used, automatic EOI is disabled, the PICs should
    // operate in non-buffered mode, and special fully nested mode is disabled.
    master_pic_data_port.write(0x01);
    slave_pic_data_port.write(0x01);
    io_wait();

    // Finally, restore the original masks that were saved before sending ICW1.
    master_pic_data_port.write(master_mask);
    slave_pic_data_port.write(slave_mask);
}

pub unsafe fn mask_all_irqs() {
    // Mask all interrupts except IRQ2. IRQ2 is used for communicating between the master and slave PICs, so it should not be masked or the
    // slave PIC won't work correctly.
    Port::new(MASTER_PIC_DATA_PORT).write(0xfb_u8);
    Port::new(SLAVE_PIC_DATA_PORT).write(0xff_u8);
}

pub unsafe fn set_irq_masked(irq: u8, masked: bool) {
    assert!(irq < 0x10);

    let mut pic_data_port: Port<u8> = Port::new(if irq > 7 { SLAVE_PIC_DATA_PORT } else { MASTER_PIC_DATA_PORT });
    let imr = pic_data_port.read();
    let mask_bit = 1 << (irq & 0x7);

    pic_data_port.write(if masked { imr | mask_bit } else { imr & !mask_bit });
}

pub fn read_isr() -> u16 {
    unsafe {
        Port::new(MASTER_PIC_COMMAND_PORT).write(0x0A_u8);
        Port::new(SLAVE_PIC_COMMAND_PORT).write(0x0A_u8);

        let mut master_data_port: Port<u8> = Port::new(MASTER_PIC_DATA_PORT);
        let mut slave_data_port: Port<u8> = Port::new(SLAVE_PIC_DATA_PORT);

        (master_data_port.read() as u16) | ((slave_data_port.read() as u16) << 8)
    }
}

pub unsafe fn send_eoi(irq: u8) {
    assert!(irq < 0x10);

    // If the IRQ came from the slave PIC, then start by sending an EOI to it.
    if irq > 0x7 {
        Port::new(SLAVE_PIC_COMMAND_PORT).write(0x20_u8);
    };

    // Regardless of whether the IRQ came from the master PIC itself, it still needs an EOI. If the IRQ was sent from the slave PIC, then
    // this will acknowledge the IRQ2 that the slave PIC used to tell the master PIC about the interrupt.
    Port::new(MASTER_PIC_COMMAND_PORT).write(0x20_u8);
}
