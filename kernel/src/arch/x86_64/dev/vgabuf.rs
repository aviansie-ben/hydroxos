use core::fmt;

use dyn_dyn::dyn_dyn_impl;
use x86_64::instructions::port::Port;

use crate::arch::page;
use crate::arch::PhysAddr;
use crate::io::ansi::AnsiColor;
use crate::io::dev::Device;
use crate::io::vt::{TerminalDisplay, VTChar, VirtualTerminalInternals};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0x0,
    Blue = 0x1,
    Green = 0x2,
    Cyan = 0x3,
    Red = 0x4,
    Magenta = 0x5,
    Brown = 0x6,
    LightGray = 0x7,
    DarkGray = 0x8,
    LightBlue = 0x9,
    LightGreen = 0xa,
    LightCyan = 0xb,
    LightRed = 0xc,
    Pink = 0xd,
    Yellow = 0xe,
    White = 0xf
}

impl Color {
    pub fn from_ansi_color(color: AnsiColor) -> Color {
        match color {
            AnsiColor::Black => Color::Black,
            AnsiColor::Blue => Color::Blue,
            AnsiColor::Green => Color::Green,
            AnsiColor::Cyan => Color::Cyan,
            AnsiColor::Red => Color::Red,
            AnsiColor::Magenta => Color::Magenta,
            AnsiColor::Brown => Color::Brown,
            AnsiColor::LightGray => Color::LightGray,
            AnsiColor::DarkGray => Color::DarkGray,
            AnsiColor::LightBlue => Color::LightBlue,
            AnsiColor::LightGreen => Color::LightGreen,
            AnsiColor::LightCyan => Color::LightCyan,
            AnsiColor::LightRed => Color::LightRed,
            AnsiColor::Pink => Color::Pink,
            AnsiColor::Yellow => Color::Yellow,
            AnsiColor::White => Color::White
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    fn new(fg: Color, bg: Color) -> ColorCode {
        ColorCode((bg as u8) << 4 | (fg as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ch: u8,
    color: ColorCode
}

#[derive(Debug)]
pub struct VgaTextBuffer {
    buf: *mut ScreenChar,
    width: usize,
    height: usize
}

impl VgaTextBuffer {
    pub unsafe fn new(buf: *mut u8, width: usize, height: usize) -> VgaTextBuffer {
        VgaTextBuffer {
            buf: buf as *mut ScreenChar,
            width,
            height
        }
    }

    pub unsafe fn for_primary_display() -> VgaTextBuffer {
        VgaTextBuffer::new(page::get_phys_mem_ptr_mut(PhysAddr::new(0xb8000)), 80, 25)
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn clear(&mut self, fg_color: Color, bg_color: Color) {
        let clear_char = ScreenChar {
            ch: b' ',
            color: ColorCode::new(fg_color, bg_color)
        };

        for i in 0..(self.width * self.height) {
            unsafe {
                core::ptr::write_volatile(self.buf.add(i), clear_char);
            };
        }
    }

    pub fn set(&mut self, x: usize, y: usize, ch: u8, fg_color: Color, bg_color: Color) {
        assert!(x < self.width);
        assert!(y < self.height);

        unsafe {
            core::ptr::write_volatile(self.buf.add(y * self.width + x), ScreenChar {
                ch,
                color: ColorCode::new(fg_color, bg_color)
            });
        }
    }

    fn copy(&mut self, from_x: usize, from_y: usize, to_x: usize, to_y: usize) {
        assert!(from_x < self.width);
        assert!(from_y < self.height);
        assert!(to_x < self.width);
        assert!(to_y < self.height);

        unsafe {
            core::ptr::write_volatile(
                self.buf.add(to_y * self.width + to_x),
                core::ptr::read_volatile(self.buf.add(from_y * self.width + from_y))
            );
        }
    }

    fn move_cursor_internal(&mut self, pos: usize) {
        let mut index_reg: Port<u8> = Port::new(0x3d4);
        let mut data_reg: Port<u8> = Port::new(0x3d5);

        unsafe {
            index_reg.write(0x0f);
            data_reg.write(pos as u8);

            index_reg.write(0x0e);
            data_reg.write((pos >> 8) as u8);
        };
    }

    pub fn move_cursor(&mut self, x: usize, y: usize) {
        assert!(x < self.width);
        assert!(y < self.height);

        self.move_cursor_internal(y * self.width + x);
    }

    pub fn hide_cursor(&mut self) {
        self.move_cursor_internal(self.width * self.height);
    }
}

unsafe impl Send for VgaTextBuffer {}
unsafe impl Sync for VgaTextBuffer {}

impl TerminalDisplay for VgaTextBuffer {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn clear(&mut self) {
        self.clear(Color::White, Color::Black);
    }

    fn redraw(&mut self, vt: &VirtualTerminalInternals) {
        for y in 0..vt.size.1.min(self.height) {
            for x in 0..vt.size.0.min(self.width) {
                let VTChar { ch, fg_color, bg_color } = vt.buf[vt.off(x, y)];
                let ch = if ch.is_ascii() && !ch.is_ascii_control() {
                    ch as u8
                } else {
                    b'\xfe'
                };

                self.set(x, y, ch, Color::from_ansi_color(fg_color), Color::from_ansi_color(bg_color));
            }
        }

        if vt.cursor_hidden {
            self.hide_cursor();
        } else {
            self.move_cursor(vt.cursor_pos.0, vt.cursor_pos.1);
        };
    }
}

#[dyn_dyn_impl(TerminalDisplay)]
impl Device for VgaTextBuffer {}

pub struct Writer<'a> {
    x: usize,
    y: usize,
    fg_color: Color,
    bg_color: Color,
    buf: &'a mut VgaTextBuffer
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut VgaTextBuffer) -> Self {
        Writer {
            x: 0,
            y: 0,
            fg_color: Color::White,
            bg_color: Color::Black,
            buf
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear(self.fg_color, self.bg_color);

        self.x = 0;
        self.y = 0;
        self.buf.move_cursor(0, 0);
    }

    pub fn set_position(&mut self, x: usize, y: usize) {
        self.x = x.min(self.buf.width - 1);
        self.y = y.min(self.buf.height - 1);
        self.buf.move_cursor(self.x, self.y);
    }

    pub fn set_color(&mut self, fg_color: Color, bg_color: Color) {
        self.fg_color = fg_color;
        self.bg_color = bg_color;
    }

    fn new_line(&mut self) {
        self.x = 0;
        self.y += 1;

        if self.y >= self.buf.height {
            for y in 0..(self.buf.height - 1) {
                for x in 0..self.buf.width {
                    self.buf.copy(x, y + 1, x, y);
                }
            }

            for x in 0..self.buf.width {
                self.buf.set(x, self.buf.height - 1, b' ', self.fg_color, self.bg_color);
            }

            self.y = self.buf.height - 1;
        };
    }

    fn write_char_impl(&mut self, ch: char) {
        if ch == '\n' {
            self.new_line();
        } else {
            let ch = if ch.is_ascii() && !ch.is_ascii_control() {
                ch as u8
            } else {
                b'\xfe'
            };

            self.buf.set(self.x, self.y, ch, self.fg_color, self.bg_color);

            self.x += 1;
            if self.x >= self.buf.width {
                self.new_line();
            };
        };
    }

    pub fn write_char(&mut self, ch: char) {
        self.write_char_impl(ch);
        self.buf.move_cursor(self.x, self.y);
    }

    pub fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char_impl(ch);
        }
        self.buf.move_cursor(self.x, self.y);
    }
}

impl<'a> fmt::Write for Writer<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }
}
