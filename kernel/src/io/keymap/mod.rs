use core::mem;

use super::dev::kbd::{KeyboardLockState, ModifierState};

mod qwerty_us;

#[derive(Debug)]
pub struct InvalidKeycodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CommonKeycode {
    Esc,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Tilde,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Num0,
    Minus,
    Equal,
    Backspace,
    Tab,
    Q,
    W,
    E,
    R,
    T,
    Y,
    U,
    I,
    O,
    P,
    LeftBracket,
    RightBracket,
    Backslash,
    CapsLock,
    A,
    S,
    D,
    F,
    G,
    H,
    J,
    K,
    L,
    Colon,
    Quote,
    Enter,
    LeftShift,
    Z,
    X,
    C,
    V,
    B,
    N,
    M,
    Comma,
    Period,
    Slash,
    RightShift,
    LeftCtrl,
    LeftSuper,
    LeftAlt,
    Space,
    RightAlt,
    RightSuper,
    Menu,
    RightCtrl,
    PrintScreen,
    ScrollLock,
    Pause,
    Insert,
    Home,
    PageUp,
    Delete,
    End,
    PageDown,
    UpArrow,
    LeftArrow,
    DownArrow,
    RightArrow,
    NumLock,
    NumpadSlash,
    NumpadTimes,
    NumpadMinus,
    NumpadPlus,
    NumpadDot,
    NumpadEnter,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9
}

impl CommonKeycode {
    pub const NUM_KEYCODES: usize = CommonKeycode::Numpad9 as usize + 1;
}

impl TryFrom<u8> for CommonKeycode {
    type Error = InvalidKeycodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (value as usize) < Self::NUM_KEYCODES {
            // SAFETY: All u8 values less than NUM_KEYCODES have a valid enum representation
            Ok(unsafe { mem::transmute(value) })
        } else {
            Err(InvalidKeycodeError)
        }
    }
}

impl TryFrom<usize> for CommonKeycode {
    type Error = InvalidKeycodeError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u8::try_from(value)
            .map_err(|_| InvalidKeycodeError)
            .and_then(CommonKeycode::try_from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keycode {
    Common(CommonKeycode),
    DeviceSpecific(u16)
}

impl TryFrom<usize> for Keycode {
    type Error = InvalidKeycodeError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(if value < 0x10000 {
            Keycode::Common(CommonKeycode::try_from(value)?)
        } else {
            Keycode::DeviceSpecific(u16::try_from(value - 0x10000).map_err(|_| InvalidKeycodeError)?)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeycodeMapEntry {
    Simple(Option<char>),
    Shift(Option<char>, Option<char>),
    ShiftCaps(Option<char>, Option<char>),
    NumLock(Option<char>, Option<char>)
}

#[derive(Debug)]
pub struct KeycodeMap {
    name: &'static str,
    common: [KeycodeMapEntry; CommonKeycode::NUM_KEYCODES]
}

impl KeycodeMap {
    pub fn fallback() -> &'static Self {
        &qwerty_us::KEYMAP
    }

    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            common: [KeycodeMapEntry::Simple(None); CommonKeycode::NUM_KEYCODES]
        }
    }

    pub const fn set_common(&mut self, k: CommonKeycode, e: KeycodeMapEntry) {
        self.common[k as usize] = e;
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn get(&self, k: Keycode, lock_state: KeyboardLockState, mod_state: ModifierState) -> Option<char> {
        if mod_state.ctrl() || mod_state.alt() || mod_state.super_key() {
            return None;
        }

        match k {
            Keycode::Common(k) => match self.common[k as usize] {
                KeycodeMapEntry::Simple(ch) => ch,
                KeycodeMapEntry::Shift(ch_false, ch_true) => {
                    if mod_state.shift() {
                        ch_true
                    } else {
                        ch_false
                    }
                },
                KeycodeMapEntry::ShiftCaps(ch_false, ch_true) => {
                    if mod_state.shift() != lock_state.caps_lock {
                        ch_true
                    } else {
                        ch_false
                    }
                },
                KeycodeMapEntry::NumLock(ch_false, ch_true) => {
                    if lock_state.num_lock {
                        ch_true
                    } else {
                        ch_false
                    }
                },
            },
            Keycode::DeviceSpecific(_) => None
        }
    }
}

pub fn get_keymap(name: &str) -> Option<&'static KeycodeMap> {
    match name {
        "qwerty-us" => Some(&qwerty_us::KEYMAP),
        _ => None
    }
}
