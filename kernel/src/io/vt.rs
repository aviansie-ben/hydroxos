use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::iter::FromIterator;

use dyn_dyn::dyn_dyn_impl;

use super::dev::hub::{DeviceHub, DeviceHubLockedError};
use super::dev::{Device, DeviceNode};
use crate::io::ansi::{AnsiColor, AnsiParser, AnsiParserAction, AnsiParserSgrAction};
use crate::io::dev::{device_root, DeviceRef};
use crate::io::tty::Tty;
use crate::sync::{Future, UninterruptibleSpinlock};
use crate::util::SharedUnsafeCell;

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
        let vtmgr = get_global_manager().dev().internal.lock();
        for &(ref display, vt_id) in vtmgr.displays.iter() {
            if vt_id == self.id {
                display.dev().redraw(self);
            };
        }
    }
}

#[derive(Debug)]
pub struct VirtualTerminal(UninterruptibleSpinlock<VirtualTerminalInternals>);

impl Tty for VirtualTerminal {
    unsafe fn write(&self, bytes: *const [u8]) -> Future<Result<(), ()>> {
        let mut vt = self.0.lock();
        for i in 0..bytes.len() {
            vt.write_byte((*bytes)[i]);
        }

        vt.redraw();
        Future::done(Ok(()))
    }

    unsafe fn flush(&self) -> Future<Result<(), ()>> {
        Future::done(Ok(()))
    }

    unsafe fn read(&self, _: *mut [u8]) -> Future<Result<usize, ()>> {
        Future::done(Err(()))
    }
}

#[dyn_dyn_impl(Tty)]
impl Device for VirtualTerminal {}

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

pub trait TerminalDisplay: Device {
    fn size(&self) -> (usize, usize);
    fn clear(&self);
    fn redraw(&self, vt: &VirtualTerminalInternals);
}

#[derive(Debug)]
struct VirtualTerminalManagerInternals {
    terminals: Vec<DeviceRef<VirtualTerminal>>,
    displays: Vec<(DeviceRef<dyn TerminalDisplay>, usize)>
}

impl VirtualTerminalManagerInternals {
    unsafe fn on_connected(&mut self, own_ref: &DeviceRef<VirtualTerminalManager>) {
        assert!(!self.displays.is_empty());

        let (width, height) = self.displays[0].0.dev().size();

        self.terminals.push(
            DeviceNode::new(Box::from("vt0"), VirtualTerminal::new(0, width, height))
                .connect(DeviceRef::<VirtualTerminalManager>::downgrade(own_ref))
        );

        self.displays[0].0.dev().redraw(&self.terminals[0].dev().0.lock());
    }

    unsafe fn on_disconnected(&mut self) {
        for t in self.terminals.iter() {
            t.disconnect();
        }

        self.terminals = vec![];
        self.displays = vec![];
    }

    fn for_terminals(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> bool {
        for t in self.terminals.iter() {
            let t: DeviceRef<dyn Device> = t.clone();
            if !f(&t) {
                return false;
            }
        }

        true
    }
}

#[derive(Debug)]
pub struct VirtualTerminalManager {
    internal: UninterruptibleSpinlock<VirtualTerminalManagerInternals>
}

impl VirtualTerminalManager {
    fn new(primary_display: DeviceRef<dyn TerminalDisplay>) -> VirtualTerminalManager {
        VirtualTerminalManager {
            internal: UninterruptibleSpinlock::new(VirtualTerminalManagerInternals {
                terminals: vec![],
                displays: vec![(primary_display, 0)]
            })
        }
    }

    pub fn get_terminal(&self, id: usize) -> Option<DeviceRef<VirtualTerminal>> {
        self.internal.lock().terminals.get(id).cloned()
    }

    pub fn switch_display(&self, display_id: usize, terminal_id: usize) -> bool {
        let mut vtmgr = self.internal.lock();

        if terminal_id < vtmgr.terminals.len() {
            let vtmgr = &mut *vtmgr;
            let vt = vtmgr.terminals[terminal_id].dev().0.lock();

            if display_id < vtmgr.displays.len() {
                vtmgr.displays[display_id].1 = terminal_id;
                vtmgr.displays[display_id].0.dev().redraw(&*vt);
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

impl DeviceHub for VirtualTerminalManager {
    fn for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> bool {
        self.internal.lock().for_terminals(f)
    }

    fn try_for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> Result<bool, DeviceHubLockedError> {
        match self.internal.try_lock() {
            Some(internal) => Ok(internal.for_terminals(f)),
            None => Err(DeviceHubLockedError)
        }
    }
}

#[dyn_dyn_impl(DeviceHub)]
impl Device for VirtualTerminalManager {
    unsafe fn on_connected(&self, own_ref: &DeviceRef<VirtualTerminalManager>) {
        self.internal.lock().on_connected(own_ref);
    }

    unsafe fn on_disconnected(&self) {
        self.internal.lock().on_disconnected();
    }
}

static VT_MANAGER: SharedUnsafeCell<Option<DeviceRef<VirtualTerminalManager>>> = SharedUnsafeCell::new(None);

pub unsafe fn init(primary_display: DeviceRef<dyn TerminalDisplay>) {
    assert!((*VT_MANAGER.get()).is_none());

    *VT_MANAGER.get() = Some(
        device_root()
            .dev()
            .add_device(DeviceNode::new(Box::from("vtmgr"), VirtualTerminalManager::new(primary_display)))
    );
}

pub fn get_global_manager() -> &'static DeviceRef<VirtualTerminalManager> {
    unsafe { (*VT_MANAGER.get()).as_ref().unwrap() }
}
