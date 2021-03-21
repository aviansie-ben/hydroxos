use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::iter::FromIterator;

use crate::x86_64::dev::vgabuf;
use crate::io::tty::Tty;
use crate::future::Future;
use crate::util::InterruptDisableSpinlock;

const MAX_CSI_LENGTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VTChar {
    ch: char,
    fg_color: vgabuf::Color,
    bg_color: vgabuf::Color
}

#[derive(Debug)]
enum VirtualTerminalState {
    Normal,
    PartialUtf8(usize, usize),
    Escape,
    PartialCsi(usize)
}

#[derive(Debug)]
struct VirtualTerminalInternals {
    buf: Box<[VTChar]>,
    buf_line: usize,
    size: (usize, usize),
    state: VirtualTerminalState,
    partial_buf: [u8; MAX_CSI_LENGTH],
    cursor_pos: (usize, usize),
    fg_color: vgabuf::Color,
    bg_color: vgabuf::Color,
    cursor_hidden: bool,
    id: usize
}

impl VirtualTerminalInternals {
    fn off(&self, x: usize, y: usize) -> usize {
        let (w, h) = self.size;

        let y = y + self.buf_line;
        let y = if y >= h {
            y - h
        } else {
            y
        };

        y * w + x
    }

    fn buf_end(&self) -> usize {
        let (w, h) = self.size;

        w * h
    }

    fn cursor_off(&self) -> usize {
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
            };
        } else {
            for i in start..self.buf_end() {
                self.buf[i] = clear_char;
            };

            for i in 0..end {
                self.buf[i] = clear_char;
            };
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
        match self.state {
            VirtualTerminalState::Normal => match b {
                b'\x1b' => {
                    self.state = VirtualTerminalState::Escape;
                },
                b'\xc0'..=b'\xdf' => {
                    self.partial_buf[0] = b;
                    self.state = VirtualTerminalState::PartialUtf8(1, 2);
                },
                b'\xe0'..=b'\xef' => {
                    self.partial_buf[0] = b;
                    self.state = VirtualTerminalState::PartialUtf8(1, 3);
                },
                b'\xf0'..=b'\xff' => {
                    self.partial_buf[0] = b;
                    self.state = VirtualTerminalState::PartialUtf8(1, 4);
                },
                _ => {
                    self.write_char(b as char);
                }
            },
            VirtualTerminalState::PartialUtf8(i, len) => {
                self.partial_buf[i] = b;

                if i + 1 == len {
                    self.write_char(if let Ok(s) = core::str::from_utf8(&self.partial_buf[0..len]) {
                        s.chars().next().unwrap()
                    } else {
                        '\u{fffd}'
                    });
                    self.state = VirtualTerminalState::Normal;
                } else {
                    self.state = VirtualTerminalState::PartialUtf8(i + 1, len);
                };
            },
            VirtualTerminalState::Escape => match b {
                b'[' => {
                    self.state = VirtualTerminalState::PartialCsi(0);
                },
                _ => {
                    self.state = VirtualTerminalState::Normal;
                }
            },
            VirtualTerminalState::PartialCsi(MAX_CSI_LENGTH) => match b {
                b'@'..b'~' => {
                    self.state = VirtualTerminalState::Normal;
                },
                _ => {}
            },
            VirtualTerminalState::PartialCsi(i) => {
                self.partial_buf[i] = b;

                match b {
                    b'@'..b'~' => {
                        // TODO Execute CSI
                        self.state = VirtualTerminalState::Normal;
                    },
                    _ => {
                        self.state = VirtualTerminalState::PartialCsi(i + 1);
                    }
                }
            }
        }
    }

    fn redraw(&self) {
        VIRTUAL_DISPLAYS.with_lock(|virtual_displays| {
            for &mut (ref mut display, vt_id) in virtual_displays.iter_mut() {
                if vt_id == self.id {
                    display.redraw(self);
                };
            };
        });
    }
}

#[derive(Debug)]
pub struct VirtualTerminal(InterruptDisableSpinlock<VirtualTerminalInternals>);

impl Tty for VirtualTerminal {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>> {
        self.0.with_lock(|vt| {
            for i in 0..bytes.len() {
                vt.write_byte((*bytes)[i]);
            };

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

        VirtualTerminal(InterruptDisableSpinlock::new(VirtualTerminalInternals {
            buf: Vec::from_iter(itertools::repeat_n(VTChar {
                ch: ' ',
                fg_color: vgabuf::Color::White,
                bg_color: vgabuf::Color::Black
            }, width * height)).into_boxed_slice(),
            buf_line: 0,
            size: (width, height),
            state: VirtualTerminalState::Normal,
            partial_buf: [0; MAX_CSI_LENGTH],
            cursor_pos: (0, 0),
            fg_color: vgabuf::Color::White,
            bg_color: vgabuf::Color::Black,
            cursor_hidden: false,
            id
        }))
    }
}

pub enum VirtualTerminalDisplay {
    VgaText(vgabuf::TextBuffer)
}

impl VirtualTerminalDisplay {
    pub fn size(&self) -> (usize, usize) {
        match *self {
            VirtualTerminalDisplay::VgaText(ref buf) => buf.size()
        }
    }

    pub fn clear(&mut self) {
        match *self {
            VirtualTerminalDisplay::VgaText(ref mut buf) => {
                buf.clear(vgabuf::Color::White, vgabuf::Color::Black);
            }
        };
    }

    fn redraw(&mut self, term: &VirtualTerminalInternals) {
        match *self {
            VirtualTerminalDisplay::VgaText(ref mut buf) => {
                for y in 0..term.size.1.min(buf.size().1) {
                    for x in 0..term.size.0.min(buf.size().0) {
                        let VTChar { ch, fg_color, bg_color } = term.buf[term.off(x, y)];
                        let ch = if ch.is_ascii() && !ch.is_ascii_control() {
                            ch as u8
                        } else {
                            b'\xfe'
                        };

                        buf.set(x, y, ch, fg_color, bg_color);
                    };
                };

                if term.cursor_hidden {
                    buf.hide_cursor();
                } else {
                    buf.move_cursor(term.cursor_pos.0, term.cursor_pos.1);
                };
            }
        };
    }
}

static VIRTUAL_TERMINALS: InterruptDisableSpinlock<Vec<Arc<VirtualTerminal>>> = InterruptDisableSpinlock::new(Vec::new());
static VIRTUAL_DISPLAYS: InterruptDisableSpinlock<Vec<(VirtualTerminalDisplay, usize)>> = InterruptDisableSpinlock::new(Vec::new());

pub fn init(primary_display: VirtualTerminalDisplay, num_terminals: usize) {
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
        };

        virtual_terminals[0].0.with_lock(|vt| {
            VIRTUAL_DISPLAYS.with_lock(|virtual_displays| {
                virtual_displays[0].0.redraw(vt);
            });
        });
    });
}

pub fn get_terminal(id: usize) -> Option<Arc<VirtualTerminal>> {
    VIRTUAL_TERMINALS.with_lock(|virtual_terminals| {
        virtual_terminals.get(id).cloned()
    })
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
