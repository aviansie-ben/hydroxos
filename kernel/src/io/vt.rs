use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::iter::FromIterator;

use crate::io::ansi::{AnsiColor, AnsiParser, AnsiParserAction, AnsiParserSgrAction};
use crate::io::tty::Tty;
use crate::sync::{Future, UninterruptibleSpinlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VTChar {
    pub ch: char,
    pub fg_color: AnsiColor,
    pub bg_color: AnsiColor
}

#[derive(Debug)]
pub struct VirtualTerminalInternals {
    pub buf: Box<[VTChar]>,
    pub buf_line: usize,
    pub size: (usize, usize),
    ansi: AnsiParser,
    pub cursor_pos: (usize, usize),
    pub fg_color: AnsiColor,
    pub bg_color: AnsiColor,
    pub cursor_hidden: bool,
    id: usize
}

impl VirtualTerminalInternals {
    pub fn off(&self, x: usize, y: usize) -> usize {
        let (w, h) = self.size;

        let y = y + self.buf_line;
        let y = if y >= h { y - h } else { y };

        y * w + x
    }

    pub fn buf_end(&self) -> usize {
        let (w, h) = self.size;

        w * h
    }

    pub fn cursor_off(&self) -> usize {
        let (x, y) = self.cursor_pos;
        self.off(x, y)
    }

    fn new_line(&mut self) {
        self.cursor_pos.0 = 0;
        self.cursor_pos.1 += 1;

        if self.cursor_pos.1 >= self.size.1 {
            self.scroll_up(1);
        };
    }

    fn clear_range(&mut self, start: usize, end: usize) {
        let clear_char = VTChar {
            ch: ' ',
            fg_color: self.fg_color,
            bg_color: self.bg_color
        };

        assert!(start <= self.buf.len());
        assert!(end <= self.buf.len());

        if start <= end {
            for i in start..end {
                self.buf[i] = clear_char;
            }
        } else {
            for i in start..self.buf_end() {
                self.buf[i] = clear_char;
            }

            for i in 0..end {
                self.buf[i] = clear_char;
            }
        };
    }

    fn clear(&mut self) {
        self.clear_range(0, self.buf_end());
    }

    fn scroll_up(&mut self, n: usize) {
        let (_, h) = self.size;

        if n >= h {
            self.clear();
            self.cursor_pos = (0, 0);
        } else {
            self.cursor_pos = (0, self.cursor_pos.1.saturating_sub(n));
            self.buf_line += n;

            if self.buf_line >= h {
                self.buf_line -= h;
            };

            self.clear_range(self.off(0, h - n), self.off(0, h));
        };
    }

    fn write_char(&mut self, ch: char) {
        match ch {
            '\n' => {
                self.new_line();
            },
            '\x00'..='\x1f' | '\x7f' => {},
            _ => {
                self.buf[self.cursor_off()] = VTChar {
                    ch,
                    fg_color: self.fg_color,
                    bg_color: self.bg_color
                };

                self.cursor_pos.0 += 1;
                if self.cursor_pos.0 >= self.size.0 {
                    self.new_line();
                };
            }
        }
    }

    fn write_byte(&mut self, b: u8) {
        match self.ansi.write(b) {
            Some(AnsiParserAction::WriteChar(ch)) => {
                self.write_char(ch);
            },
            Some(AnsiParserAction::Sgr(sgr, sgr_len)) => {
                for &sgr in sgr[0..sgr_len].iter() {
                    match sgr {
                        AnsiParserSgrAction::Reset => {
                            self.fg_color = AnsiColor::White;
                            self.bg_color = AnsiColor::Black;
                        },
                        AnsiParserSgrAction::SetFgColor(color) => {
                            self.fg_color = color;
                        },
                        AnsiParserSgrAction::SetBgColor(color) => {
                            self.bg_color = color;
                        }
                    }
                }
            },
            None => {}
        }
    }

    fn redraw(&self) {
        VIRTUAL_DISPLAYS.with_lock(|virtual_displays| {
            for &mut (ref mut display, vt_id) in virtual_displays.iter_mut() {
                if vt_id == self.id {
                    display.redraw(self);
                };
            }
        });
    }
}

#[derive(Debug)]
pub struct VirtualTerminal(UninterruptibleSpinlock<VirtualTerminalInternals>);

impl Tty for VirtualTerminal {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>> {
        self.0.with_lock(|vt| {
            for i in 0..bytes.len() {
                vt.write_byte((*bytes)[i]);
            }

            vt.redraw();
            Future::done(Ok(()))
        })
    }

    unsafe fn flush(&self) -> Future<Result<(), ()>> {
        Future::done(Ok(()))
    }

    unsafe fn read(&self, _: *mut [u8]) -> Future<Result<usize, ()>> {
        Future::done(Err(()))
    }
}

impl VirtualTerminal {
    pub fn new(id: usize, width: usize, height: usize) -> VirtualTerminal {
        assert!(width > 0);
        assert!(height > 0);
        assert!(width.checked_mul(height).is_some());

        VirtualTerminal(UninterruptibleSpinlock::new(VirtualTerminalInternals {
            buf: Vec::from_iter(itertools::repeat_n(
                VTChar {
                    ch: ' ',
                    fg_color: AnsiColor::White,
                    bg_color: AnsiColor::Black
                },
                width * height
            ))
            .into_boxed_slice(),
            buf_line: 0,
            size: (width, height),
            ansi: AnsiParser::new(),
            cursor_pos: (0, 0),
            fg_color: AnsiColor::White,
            bg_color: AnsiColor::Black,
            cursor_hidden: false,
            id
        }))
    }
}

pub trait VirtualTerminalDisplay: Send {
    fn size(&self) -> (usize, usize);
    fn clear(&mut self);
    fn redraw(&mut self, vt: &VirtualTerminalInternals);
}

static VIRTUAL_TERMINALS: UninterruptibleSpinlock<Vec<Arc<VirtualTerminal>>> = UninterruptibleSpinlock::new(Vec::new());
static VIRTUAL_DISPLAYS: UninterruptibleSpinlock<Vec<(Box<dyn VirtualTerminalDisplay>, usize)>> = UninterruptibleSpinlock::new(Vec::new());

pub fn init(primary_display: Box<dyn VirtualTerminalDisplay>, num_terminals: usize) {
    assert!(num_terminals > 0);

    let (width, height) = primary_display.size();

    VIRTUAL_DISPLAYS.with_lock(|virtual_displays| {
        assert!(virtual_displays.is_empty());

        virtual_displays.reserve_exact(1);
        virtual_displays.push((primary_display, 0));
    });
    VIRTUAL_TERMINALS.with_lock(|virtual_terminals| {
        assert!(virtual_terminals.is_empty());

        virtual_terminals.reserve_exact(num_terminals);
        for i in 0..num_terminals {
            virtual_terminals.push(Arc::new(VirtualTerminal::new(i, width, height)));
        }

        virtual_terminals[0].0.with_lock(|vt| {
            VIRTUAL_DISPLAYS.with_lock(|virtual_displays| {
                virtual_displays[0].0.redraw(vt);
            });
        });
    });
}

pub fn get_terminal(id: usize) -> Option<Arc<VirtualTerminal>> {
    VIRTUAL_TERMINALS.with_lock(|virtual_terminals| virtual_terminals.get(id).cloned())
}

pub fn switch_display(display_id: usize, terminal_id: usize) -> bool {
    VIRTUAL_TERMINALS.with_lock(|virtual_terminals| {
        if terminal_id < virtual_terminals.len() {
            virtual_terminals[terminal_id].0.with_lock(|vt| {
                VIRTUAL_DISPLAYS.with_lock(|virtual_displays| {
                    if display_id < virtual_displays.len() {
                        virtual_displays[display_id].1 = terminal_id;
                        virtual_displays[display_id].0.redraw(vt);
                        true
                    } else {
                        false
                    }
                })
            })
        } else {
            false
        }
    })
}
