use core::mem;

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
