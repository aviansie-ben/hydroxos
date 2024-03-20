use alloc::boxed::Box;

use dyn_dyn::dyn_dyn_impl;

use crate::arch::{interrupt, pic};
use crate::io::dev::kbd::{KeyPress, Keyboard, KeyboardError, KeyboardHeldKeys, KeyboardLockState, ModifierState};
use crate::io::dev::{device_root, DeviceNode};
use crate::io::dev::{hub::DeviceHub, Device, DeviceRef};
use crate::io::keymap::{CommonKeycode, Keycode, KeycodeMap};
use crate::io::vt;
use crate::sync::future::FutureWriter;
use crate::sync::uninterruptible::{UninterruptibleSpinlockGuard, UninterruptibleSpinlockReadGuard};
use crate::sync::{Future, UninterruptibleSpinlock};
use crate::util::{ArrayDeque, SharedUnsafeCell};
use crate::{log, sched};

mod scancode_2_map;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScancodeKey {
    Basic(u8),
    Extended(u8),
    DualExtended(u8, u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scancode {
    key: ScancodeKey,
    released: bool
}

impl Scancode {
    pub fn try_parse(buf: &[u8]) -> Option<(Scancode, usize)> {
        match buf.first().copied() {
            Some(0xE0) => match buf.get(1).copied() {
                Some(0xF0) if buf.len() >= 3 => Some((
                    Scancode {
                        key: ScancodeKey::Extended(buf[2]),
                        released: true
                    },
                    3
                )),
                Some(0xF0) => None,
                Some(key) => Some((
                    Scancode {
                        key: ScancodeKey::Extended(key),
                        released: false
                    },
                    2
                )),
                None => None
            },
            Some(0xE1) => match buf.get(1).copied() {
                Some(0xF0) if buf.len() >= 5 => Some((
                    Scancode {
                        key: ScancodeKey::DualExtended(buf[2], buf[4]),
                        released: true
                    },
                    5
                )),
                Some(0xF0) => None,
                _ if buf.len() >= 3 => Some((
                    Scancode {
                        key: ScancodeKey::DualExtended(buf[1], buf[2]),
                        released: false
                    },
                    3
                )),
                _ => None
            },
            Some(0xF0) if buf.len() >= 2 => Some((
                Scancode {
                    key: ScancodeKey::Basic(buf[1]),
                    released: true
                },
                2
            )),
            Some(0xF0) => None,
            Some(key) => Some((
                Scancode {
                    key: ScancodeKey::Basic(key),
                    released: false
                },
                1
            )),
            None => None
        }
    }
}

#[derive(Debug)]
enum ScancodeMapDualList {
    Static(&'static [(u8, u8, Keycode)])
}

#[derive(Debug)]
struct ScancodeMap {
    basic: [Option<Keycode>; 256],
    extended: [Option<Keycode>; 256],
    dual_extended: ScancodeMapDualList
}

impl ScancodeMap {
    pub fn get(&self, key: ScancodeKey) -> Option<Keycode> {
        match key {
            ScancodeKey::Basic(b) => self.basic[b as usize],
            ScancodeKey::Extended(b) => self.extended[b as usize],
            ScancodeKey::DualExtended(b0, b1) => match self.dual_extended {
                ScancodeMapDualList::Static(list) => list.iter().find(|&&(m0, m1, _)| m0 == b0 && m1 == b1).map(|&(_, _, k)| k)
            }
        }
    }
}

#[derive(Debug)]
pub enum Ps2Error {
    ControllerError(ps2::error::ControllerError),
    MouseError(ps2::error::MouseError),
    KeyboardError(ps2::error::KeyboardError)
}

impl From<ps2::error::ControllerError> for Ps2Error {
    fn from(err: ps2::error::ControllerError) -> Self {
        Ps2Error::ControllerError(err)
    }
}

impl From<ps2::error::MouseError> for Ps2Error {
    fn from(err: ps2::error::MouseError) -> Self {
        match err {
            ps2::error::MouseError::ControllerError(err) => Ps2Error::ControllerError(err),
            _ => Ps2Error::MouseError(err)
        }
    }
}

impl From<ps2::error::KeyboardError> for Ps2Error {
    fn from(err: ps2::error::KeyboardError) -> Self {
        match err {
            ps2::error::KeyboardError::ControllerError(err) => Ps2Error::ControllerError(err),
            _ => Ps2Error::KeyboardError(err)
        }
    }
}

struct Ps2KeyboardGuard<'a> {
    controller: UninterruptibleSpinlockGuard<'a, Ps2ControllerInternals>,
    keyboard: &'a mut Ps2KeyboardInternals
}

impl<'a> Ps2KeyboardGuard<'a> {
    pub fn controller(&mut self) -> &mut Ps2ControllerInternals {
        &mut self.controller
    }

    pub fn keyboard(&mut self) -> &mut Ps2KeyboardInternals {
        self.keyboard
    }

    pub fn into_keyboard(self) -> UninterruptibleSpinlockGuard<'a, Ps2KeyboardInternals> {
        UninterruptibleSpinlockGuard::replace_data(self.controller, self.keyboard)
    }
}

#[derive(Debug)]
struct Ps2KeyboardHeldKeys {
    held: [bool; CommonKeycode::NUM_KEYCODES]
}

impl Ps2KeyboardHeldKeys {
    pub fn new() -> Self {
        Self {
            held: [false; CommonKeycode::NUM_KEYCODES]
        }
    }
}

impl KeyboardHeldKeys for Ps2KeyboardHeldKeys {
    fn is_held(&self, k: Keycode) -> bool {
        match k {
            Keycode::Common(k) => self.held[k as usize],
            Keycode::DeviceSpecific(_) => false
        }
    }

    fn for_all_held_impl(&self, f: &mut dyn FnMut(&[Keycode])) {
        let mut buf = [Keycode::DeviceSpecific(0); 32];
        let mut len = 0_usize;

        for (i, &held) in self.held.iter().enumerate() {
            if held {
                buf[len] = Keycode::Common(CommonKeycode::try_from(i).expect("keycode out of range"));
                len += 1;

                if len == buf.len() {
                    f(&buf);
                    len = 0;
                }
            }
        }

        if len != 0 {
            f(&buf[..len]);
        }
    }
}

#[derive(Debug)]
struct Ps2KeyboardInternals {
    lock_state: KeyboardLockState,
    mod_state: ModifierState,
    held_keys: Ps2KeyboardHeldKeys,
    input_buf: ArrayDeque<KeyPress, 16>,
    input_future: Option<FutureWriter<Result<KeyPress, KeyboardError>>>,
    scancode_buf: [u8; 5],
    scancode_buf_pos: usize,
    scancode_map: &'static ScancodeMap,
    keycode_map: &'static KeycodeMap
}

#[derive(Debug)]
pub struct Ps2Keyboard {
    internal: SharedUnsafeCell<Ps2KeyboardInternals>,
    controller: DeviceRef<Ps2Controller>
}

impl Ps2Keyboard {
    fn lock(&self) -> Ps2KeyboardGuard {
        self.lock_from_controller(self.controller.dev().internal.lock())
    }

    fn lock_from_controller<'a>(&'a self, guard: UninterruptibleSpinlockGuard<'a, Ps2ControllerInternals>) -> Ps2KeyboardGuard<'a> {
        assert!(self.controller.dev().internal.is_guarded_by(&guard));
        Ps2KeyboardGuard {
            controller: guard,
            keyboard: unsafe { &mut *self.internal.get() }
        }
    }

    fn handle_key_state_changed(guard: &mut Ps2KeyboardGuard, key: Keycode, pressed: bool) {
        if let Keycode::Common(key) = key {
            guard.keyboard.held_keys.held[key as usize] = pressed;
        }

        if pressed {
            let keypress = KeyPress {
                code: key,
                lock_state: guard.keyboard.lock_state,
                mods: guard.keyboard.mod_state,
                char: guard
                    .keyboard
                    .keycode_map
                    .get(key, guard.keyboard.lock_state, guard.keyboard.mod_state)
            };

            if let Some(input_future) = guard.keyboard.input_future.take() {
                input_future.finish(Ok(keypress));
            } else {
                let _ = guard.keyboard.input_buf.push_back(keypress);
            }

            guard.keyboard.lock_state.handle_key_pressed(key);
        }

        guard.keyboard.mod_state.handle_key_state_changed(key, pressed);
    }

    fn handle_interrupt(guard: &mut Ps2KeyboardGuard) {
        match guard.controller().controller.read_data() {
            Ok(b) => {
                guard.keyboard.scancode_buf[guard.keyboard.scancode_buf_pos] = b;
                guard.keyboard.scancode_buf_pos += 1;

                match Scancode::try_parse(&guard.keyboard.scancode_buf[..guard.keyboard.scancode_buf_pos]) {
                    Some((scancode, len)) => {
                        assert_eq!(len, guard.keyboard.scancode_buf_pos);
                        guard.keyboard.scancode_buf_pos = 0;

                        if let Some(key) = guard.keyboard.scancode_map.get(scancode.key) {
                            Self::handle_key_state_changed(guard, key, !scancode.released)
                        }
                    },
                    None => {
                        assert!(guard.keyboard.scancode_buf_pos < guard.keyboard.scancode_buf.len());
                    }
                }
            },
            Err(err) => {
                log!(Error, "ps2", "Error reading data from keyboard: {:?}", err);
            }
        }
    }
}

#[dyn_dyn_impl(Keyboard)]
impl Device for Ps2Keyboard {}

impl Keyboard for Ps2Keyboard {
    fn lock_state(&self) -> Result<KeyboardLockState, KeyboardError> {
        Ok(self.lock().keyboard().lock_state)
    }

    fn set_lock_state(&self, lock_state: KeyboardLockState) -> Result<(), KeyboardError> {
        self.lock().keyboard().lock_state = lock_state;
        Ok(())
    }

    fn mod_state(&self) -> Result<ModifierState, KeyboardError> {
        Ok(self.lock().keyboard().mod_state)
    }

    fn held_keys(&self) -> Result<UninterruptibleSpinlockReadGuard<dyn KeyboardHeldKeys>, KeyboardError> {
        Ok(UninterruptibleSpinlockReadGuard::map(self.lock().into_keyboard(), |k| {
            &k.held_keys as &dyn KeyboardHeldKeys
        }))
    }

    fn keymap(&self) -> &'static KeycodeMap {
        self.lock().keyboard().keycode_map
    }

    fn set_keymap(&self, map: &'static KeycodeMap) {
        self.lock().keyboard().keycode_map = map;
    }

    fn next_key(&self) -> Future<Result<KeyPress, KeyboardError>> {
        let mut guard = self.lock().into_keyboard();
        if let Some(keypress) = guard.input_buf.pop_front() {
            Future::done(Ok(keypress))
        } else if let Some(ref input_future) = guard.input_future {
            input_future.as_future()
        } else {
            let (future, writer) = Future::new();
            guard.input_future = Some(writer);
            future
        }
    }
}

#[allow(dead_code)]
struct Ps2MouseGuard<'a> {
    controller: UninterruptibleSpinlockGuard<'a, Ps2ControllerInternals>,
    mouse: &'a mut Ps2MouseInternals
}

#[allow(dead_code)]
impl<'a> Ps2MouseGuard<'a> {
    pub fn controller(&mut self) -> &mut Ps2ControllerInternals {
        &mut self.controller
    }

    pub fn mouse(&mut self) -> &mut Ps2MouseInternals {
        self.mouse
    }
}

#[derive(Debug)]
struct Ps2MouseInternals {}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Ps2Mouse {
    internal: SharedUnsafeCell<Ps2MouseInternals>,
    controller: DeviceRef<Ps2Controller>
}

#[allow(dead_code)]
impl Ps2Mouse {
    fn lock(&self) -> Ps2MouseGuard {
        self.lock_from_controller(self.controller.dev().internal.lock())
    }

    fn lock_from_controller<'a>(&'a self, guard: UninterruptibleSpinlockGuard<'a, Ps2ControllerInternals>) -> Ps2MouseGuard<'a> {
        assert!(self.controller.dev().internal.is_guarded_by(&guard));
        Ps2MouseGuard {
            controller: guard,
            mouse: unsafe { &mut *self.internal.get() }
        }
    }

    fn handle_interrupt(guard: &mut Ps2MouseGuard) {
        match guard.controller().controller.read_data() {
            Ok(_) => {},
            Err(err) => {
                log!(Error, "ps2", "Error reading data from mouse: {:?}", err);
            }
        }
    }
}

#[dyn_dyn_impl]
impl Device for Ps2Mouse {}

#[derive(Debug)]
struct Ps2ControllerInternals {
    controller: ps2::Controller,
    keyboard: Option<DeviceRef<Ps2Keyboard>>,
    mouse: Option<DeviceRef<Ps2Mouse>>
}

#[derive(Debug)]
pub struct Ps2Controller {
    internal: UninterruptibleSpinlock<Ps2ControllerInternals>
}

#[dyn_dyn_impl(DeviceHub)]
impl Device for Ps2Controller {}

impl DeviceHub for Ps2Controller {
    fn for_children(&self, f: &mut dyn FnMut(&DeviceRef<dyn Device>) -> bool) -> bool {
        let internal = self.internal.lock();

        if let Some(keyboard) = internal.keyboard.as_ref() {
            let keyboard: DeviceRef<dyn Device> = keyboard.clone();
            if !f(&keyboard) {
                return false;
            }
        }

        true
    }
}

pub unsafe fn init() -> Option<DeviceRef<Ps2Controller>> {
    let result: Result<_, Ps2Error> = try {
        // TODO: We should really check that a PS/2 controller exists before trying to configure it
        let mut controller = ps2::Controller::with_timeout(10000);

        controller.disable_keyboard()?;
        controller.disable_mouse()?;

        let _ = controller.read_data();

        let mut config = controller.read_config()?;
        config.set(
            ps2::flags::ControllerConfigFlags::ENABLE_KEYBOARD_INTERRUPT
                | ps2::flags::ControllerConfigFlags::ENABLE_MOUSE_INTERRUPT
                | ps2::flags::ControllerConfigFlags::ENABLE_TRANSLATE,
            false
        );
        controller.write_config(config)?;

        controller.test_controller()?;
        controller.write_config(config)?;

        let has_keyboard = match controller.test_keyboard() {
            Err(err) => {
                log!(Error, "ps2", "Failed to initialize keyboard: {:?}", err);
                false
            },
            Ok(()) => {
                let result: Result<u8, ps2::error::KeyboardError> = try {
                    controller.enable_keyboard()?;
                    controller.keyboard().reset_and_self_test()?;
                    controller.keyboard().set_scancode_set(2)?;
                    controller.keyboard().get_scancode_set()?
                };

                match result {
                    Err(err) => {
                        log!(Error, "ps2", "Failed to initialize keyboard: {:?}", err);
                        controller.disable_keyboard()?;
                        false
                    },
                    Ok(2) => {
                        config.set(ps2::flags::ControllerConfigFlags::DISABLE_KEYBOARD, false);
                        config.set(ps2::flags::ControllerConfigFlags::ENABLE_KEYBOARD_INTERRUPT, true);
                        true
                    },
                    Ok(_) => {
                        log!(Error, "ps2", "Keyboard does not support scancode set 2");
                        controller.disable_keyboard()?;
                        false
                    }
                }
            }
        };

        let has_mouse = match controller.test_mouse() {
            Err(err) => {
                log!(Error, "ps2", "Failed to initialize mouse: {:?}", err);
                false
            },
            Ok(()) => {
                controller.enable_mouse()?;
                match controller.mouse().reset_and_self_test() {
                    Err(err) => {
                        log!(Error, "ps2", "Failed to initialize mouse: {:?}", err);
                        controller.disable_mouse()?;
                        false
                    },
                    Ok(()) => {
                        config.set(ps2::flags::ControllerConfigFlags::DISABLE_MOUSE, false);
                        config.set(ps2::flags::ControllerConfigFlags::ENABLE_MOUSE_INTERRUPT, true);

                        controller.mouse().enable_data_reporting()?;
                        true
                    }
                }
            }
        };

        controller.write_config(config)?;

        let controller = device_root().dev().add_device(DeviceNode::new(Box::from("ps2"), Ps2Controller {
            internal: UninterruptibleSpinlock::new(Ps2ControllerInternals {
                controller,
                keyboard: None,
                mouse: None
            })
        }));

        let keyboard = if has_keyboard {
            Some(
                DeviceNode::new(Box::from("keyboard"), Ps2Keyboard {
                    controller: controller.clone(),
                    internal: SharedUnsafeCell::new(Ps2KeyboardInternals {
                        lock_state: KeyboardLockState::none(),
                        mod_state: ModifierState::none(),
                        held_keys: Ps2KeyboardHeldKeys::new(),
                        input_buf: ArrayDeque::new(),
                        input_future: None,
                        scancode_buf: [0; 5],
                        scancode_buf_pos: 0,
                        scancode_map: &scancode_2_map::MAP,
                        keycode_map: KeycodeMap::fallback()
                    })
                })
                .connect(DeviceRef::<Ps2Controller>::downgrade(&controller))
            )
        } else {
            None
        };

        if let Some(ref keyboard) = keyboard {
            let keyboard = keyboard.clone();
            sched::enqueue_soft_interrupt(move || {
                vt::get_global_manager().dev().attach_keyboard(0, keyboard);
            });
        }

        let mouse = if has_mouse {
            Some(
                DeviceNode::new(Box::from("mouse"), Ps2Mouse {
                    controller: controller.clone(),
                    internal: SharedUnsafeCell::new(Ps2MouseInternals {})
                })
                .connect(DeviceRef::<Ps2Controller>::downgrade(&controller))
            )
        } else {
            None
        };

        let mut controller_lock = controller.dev().internal.lock();
        controller_lock.keyboard = keyboard;
        controller_lock.mouse = mouse;
        drop(controller_lock);

        if has_keyboard {
            let controller_for_keyboard_interrupt = controller.clone();
            interrupt::register_irq(
                1,
                Box::new(move |_| {
                    let mut internal = controller_for_keyboard_interrupt.dev().internal.lock();

                    if let Some(keyboard) = internal.keyboard.clone() {
                        Ps2Keyboard::handle_interrupt(&mut keyboard.dev().lock_from_controller(internal))
                    } else {
                        log!(Warning, "ps2", "Received keyboard interrupt with no keyboard attached?");
                        let _ = internal.controller.read_data();
                    }
                })
            );

            pic::set_irq_masked(1, false);
        }

        if has_mouse {
            let controller_for_mouse_interrupt = controller.clone();
            interrupt::register_irq(
                12,
                Box::new(move |_| {
                    let mut internal = controller_for_mouse_interrupt.dev().internal.lock();

                    if let Some(mouse) = internal.mouse.clone() {
                        Ps2Mouse::handle_interrupt(&mut mouse.dev().lock_from_controller(internal))
                    } else {
                        log!(Warning, "ps2", "Received mouse interrupt with no mouse attached?");
                        let _ = internal.controller.read_data();
                    }
                })
            );
            pic::set_irq_masked(12, false);
        }

        controller
    };

    match result {
        Ok(controller) => Some(controller),
        Err(err) => {
            log!(Error, "ps2", "Failed to initialize controller: {:?}", err);

            // Try to disable the PS/2 controller if possible in case we left it in a weird state
            // If this also results in an error, just do nothing since there's not much we can do
            let _: Result<_, Ps2Error> = try {
                let mut controller = ps2::Controller::with_timeout(10000);

                controller.disable_keyboard()?;
                controller.disable_mouse()?;

                let _ = controller.read_data();

                let mut config = controller.read_config()?;
                config.set(
                    ps2::flags::ControllerConfigFlags::ENABLE_KEYBOARD_INTERRUPT
                        | ps2::flags::ControllerConfigFlags::ENABLE_MOUSE_INTERRUPT
                        | ps2::flags::ControllerConfigFlags::ENABLE_TRANSLATE,
                    false
                );
                controller.write_config(config)?;
            };

            None
        }
    }
}
