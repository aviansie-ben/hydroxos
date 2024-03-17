use super::Device;
use crate::{
    io::keymap::{CommonKeycode, Keycode, KeycodeMap},
    sync::{uninterruptible::UninterruptibleSpinlockReadGuard, Future}
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardLockState {
    pub scroll_lock: bool,
    pub num_lock: bool,
    pub caps_lock: bool
}

impl KeyboardLockState {
    pub const fn none() -> KeyboardLockState {
        KeyboardLockState {
            scroll_lock: false,
            num_lock: false,
            caps_lock: false
        }
    }

    pub fn handle_key_pressed(&mut self, key: Keycode) -> bool {
        match key {
            Keycode::Common(CommonKeycode::ScrollLock) => {
                self.scroll_lock = !self.scroll_lock;
                true
            },
            Keycode::Common(CommonKeycode::NumLock) => {
                self.num_lock = !self.num_lock;
                true
            },
            Keycode::Common(CommonKeycode::CapsLock) => {
                self.caps_lock = !self.caps_lock;
                true
            },
            _ => false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierState {
    pub left_ctrl: bool,
    pub right_ctrl: bool,
    pub left_alt: bool,
    pub right_alt: bool,
    pub left_shift: bool,
    pub right_shift: bool,
    pub left_super_key: bool,
    pub right_super_key: bool
}

impl ModifierState {
    pub const fn none() -> ModifierState {
        ModifierState {
            left_ctrl: false,
            right_ctrl: false,
            left_alt: false,
            right_alt: false,
            left_shift: false,
            right_shift: false,
            left_super_key: false,
            right_super_key: false
        }
    }

    pub fn ctrl(&self) -> bool {
        self.left_ctrl || self.right_ctrl
    }

    pub fn alt(&self) -> bool {
        self.left_alt || self.right_alt
    }

    pub fn shift(&self) -> bool {
        self.left_shift || self.right_shift
    }

    pub fn super_key(&self) -> bool {
        self.left_super_key || self.right_super_key
    }

    pub fn handle_key_state_changed(&mut self, key: Keycode, pressed: bool) -> bool {
        match key {
            Keycode::Common(CommonKeycode::LeftCtrl) => {
                self.left_ctrl = pressed;
                true
            },
            Keycode::Common(CommonKeycode::RightCtrl) => {
                self.right_ctrl = pressed;
                true
            },
            Keycode::Common(CommonKeycode::LeftAlt) => {
                self.left_alt = pressed;
                true
            },
            Keycode::Common(CommonKeycode::RightAlt) => {
                self.right_alt = pressed;
                true
            },
            Keycode::Common(CommonKeycode::LeftShift) => {
                self.left_shift = pressed;
                true
            },
            Keycode::Common(CommonKeycode::RightShift) => {
                self.right_shift = pressed;
                true
            },
            Keycode::Common(CommonKeycode::LeftSuper) => {
                self.left_super_key = pressed;
                true
            },
            Keycode::Common(CommonKeycode::RightSuper) => {
                self.right_super_key = pressed;
                true
            },
            _ => false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    pub code: Keycode,
    pub lock_state: KeyboardLockState,
    pub mods: ModifierState,
    pub char: Option<char>
}

pub trait KeyboardHeldKeys {
    fn is_held(&self, k: Keycode) -> bool;
    fn for_all_held_impl(&self, f: &mut dyn FnMut(&[Keycode]));
}

pub trait KeyboardHeldKeysExt: KeyboardHeldKeys {
    fn for_all_held(&self, f: &mut impl FnMut(Keycode));
}

impl<T: KeyboardHeldKeys + ?Sized> KeyboardHeldKeysExt for T {
    fn for_all_held(&self, f: &mut impl FnMut(Keycode)) {
        self.for_all_held_impl(&mut |keys| {
            for &k in keys {
                f(k);
            }
        });
    }
}

#[derive(Debug)]
pub struct KeyboardError;

pub trait Keyboard: Device {
    fn lock_state(&self) -> Result<KeyboardLockState, KeyboardError>;
    fn set_lock_state(&self, lock_state: KeyboardLockState) -> Result<(), KeyboardError>;

    fn mod_state(&self) -> Result<ModifierState, KeyboardError>;
    fn held_keys(&self) -> Result<UninterruptibleSpinlockReadGuard<dyn KeyboardHeldKeys>, KeyboardError>;

    fn keymap(&self) -> &'static KeycodeMap;
    fn set_keymap(&self, map: &'static KeycodeMap);

    fn next_key(&self) -> Future<Result<KeyPress, KeyboardError>>;
}
