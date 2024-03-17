use super::{CommonKeycode, KeycodeMap, KeycodeMapEntry};

pub static KEYMAP: KeycodeMap = {
    let mut keymap = KeycodeMap::new("qwerty-us");

    keymap.set_common(CommonKeycode::Tilde, KeycodeMapEntry::Shift(Some('`'), Some('~')));
    keymap.set_common(CommonKeycode::Num1, KeycodeMapEntry::Shift(Some('1'), Some('!')));
    keymap.set_common(CommonKeycode::Num2, KeycodeMapEntry::Shift(Some('2'), Some('@')));
    keymap.set_common(CommonKeycode::Num3, KeycodeMapEntry::Shift(Some('3'), Some('#')));
    keymap.set_common(CommonKeycode::Num4, KeycodeMapEntry::Shift(Some('4'), Some('$')));
    keymap.set_common(CommonKeycode::Num5, KeycodeMapEntry::Shift(Some('5'), Some('%')));
    keymap.set_common(CommonKeycode::Num6, KeycodeMapEntry::Shift(Some('6'), Some('^')));
    keymap.set_common(CommonKeycode::Num7, KeycodeMapEntry::Shift(Some('7'), Some('&')));
    keymap.set_common(CommonKeycode::Num8, KeycodeMapEntry::Shift(Some('8'), Some('*')));
    keymap.set_common(CommonKeycode::Num9, KeycodeMapEntry::Shift(Some('9'), Some('(')));
    keymap.set_common(CommonKeycode::Num0, KeycodeMapEntry::Shift(Some('0'), Some(')')));
    keymap.set_common(CommonKeycode::Minus, KeycodeMapEntry::Shift(Some('-'), Some('_')));
    keymap.set_common(CommonKeycode::Equal, KeycodeMapEntry::Shift(Some('='), Some('+')));
    keymap.set_common(CommonKeycode::Backspace, KeycodeMapEntry::Simple(Some('\x08')));

    keymap.set_common(CommonKeycode::LeftBracket, KeycodeMapEntry::Shift(Some('['), Some('{')));
    keymap.set_common(CommonKeycode::RightBracket, KeycodeMapEntry::Shift(Some(']'), Some('}')));
    keymap.set_common(CommonKeycode::Backslash, KeycodeMapEntry::Shift(Some('\\'), Some('|')));
    keymap.set_common(CommonKeycode::Colon, KeycodeMapEntry::Shift(Some(';'), Some(':')));
    keymap.set_common(CommonKeycode::Quote, KeycodeMapEntry::Shift(Some('\''), Some('"')));
    keymap.set_common(CommonKeycode::Enter, KeycodeMapEntry::Simple(Some('\n')));
    keymap.set_common(CommonKeycode::Comma, KeycodeMapEntry::Shift(Some(','), Some('<')));
    keymap.set_common(CommonKeycode::Period, KeycodeMapEntry::Shift(Some('.'), Some('>')));
    keymap.set_common(CommonKeycode::Slash, KeycodeMapEntry::Shift(Some('/'), Some('?')));
    keymap.set_common(CommonKeycode::Space, KeycodeMapEntry::Simple(Some(' ')));

    keymap.set_common(CommonKeycode::A, KeycodeMapEntry::ShiftCaps(Some('a'), Some('A')));
    keymap.set_common(CommonKeycode::B, KeycodeMapEntry::ShiftCaps(Some('b'), Some('B')));
    keymap.set_common(CommonKeycode::C, KeycodeMapEntry::ShiftCaps(Some('c'), Some('C')));
    keymap.set_common(CommonKeycode::D, KeycodeMapEntry::ShiftCaps(Some('d'), Some('D')));
    keymap.set_common(CommonKeycode::E, KeycodeMapEntry::ShiftCaps(Some('e'), Some('E')));
    keymap.set_common(CommonKeycode::F, KeycodeMapEntry::ShiftCaps(Some('f'), Some('F')));
    keymap.set_common(CommonKeycode::G, KeycodeMapEntry::ShiftCaps(Some('g'), Some('G')));
    keymap.set_common(CommonKeycode::H, KeycodeMapEntry::ShiftCaps(Some('h'), Some('H')));
    keymap.set_common(CommonKeycode::I, KeycodeMapEntry::ShiftCaps(Some('i'), Some('I')));
    keymap.set_common(CommonKeycode::J, KeycodeMapEntry::ShiftCaps(Some('j'), Some('J')));
    keymap.set_common(CommonKeycode::K, KeycodeMapEntry::ShiftCaps(Some('k'), Some('K')));
    keymap.set_common(CommonKeycode::L, KeycodeMapEntry::ShiftCaps(Some('l'), Some('L')));
    keymap.set_common(CommonKeycode::M, KeycodeMapEntry::ShiftCaps(Some('m'), Some('M')));
    keymap.set_common(CommonKeycode::N, KeycodeMapEntry::ShiftCaps(Some('n'), Some('N')));
    keymap.set_common(CommonKeycode::O, KeycodeMapEntry::ShiftCaps(Some('o'), Some('O')));
    keymap.set_common(CommonKeycode::P, KeycodeMapEntry::ShiftCaps(Some('p'), Some('P')));
    keymap.set_common(CommonKeycode::Q, KeycodeMapEntry::ShiftCaps(Some('q'), Some('Q')));
    keymap.set_common(CommonKeycode::R, KeycodeMapEntry::ShiftCaps(Some('r'), Some('R')));
    keymap.set_common(CommonKeycode::S, KeycodeMapEntry::ShiftCaps(Some('s'), Some('S')));
    keymap.set_common(CommonKeycode::T, KeycodeMapEntry::ShiftCaps(Some('t'), Some('T')));
    keymap.set_common(CommonKeycode::U, KeycodeMapEntry::ShiftCaps(Some('u'), Some('U')));
    keymap.set_common(CommonKeycode::V, KeycodeMapEntry::ShiftCaps(Some('v'), Some('V')));
    keymap.set_common(CommonKeycode::W, KeycodeMapEntry::ShiftCaps(Some('w'), Some('W')));
    keymap.set_common(CommonKeycode::X, KeycodeMapEntry::ShiftCaps(Some('x'), Some('X')));
    keymap.set_common(CommonKeycode::Y, KeycodeMapEntry::ShiftCaps(Some('y'), Some('Y')));
    keymap.set_common(CommonKeycode::Z, KeycodeMapEntry::ShiftCaps(Some('z'), Some('Z')));

    keymap.set_common(CommonKeycode::NumpadSlash, KeycodeMapEntry::Simple(Some('/')));
    keymap.set_common(CommonKeycode::NumpadTimes, KeycodeMapEntry::Simple(Some('*')));
    keymap.set_common(CommonKeycode::NumpadMinus, KeycodeMapEntry::Simple(Some('-')));
    keymap.set_common(CommonKeycode::NumpadPlus, KeycodeMapEntry::Simple(Some('+')));
    keymap.set_common(CommonKeycode::NumpadDot, KeycodeMapEntry::NumLock(None, Some('.')));
    keymap.set_common(CommonKeycode::NumpadEnter, KeycodeMapEntry::Simple(Some('\n')));

    keymap.set_common(CommonKeycode::Numpad0, KeycodeMapEntry::NumLock(None, Some('0')));
    keymap.set_common(CommonKeycode::Numpad1, KeycodeMapEntry::NumLock(None, Some('1')));
    keymap.set_common(CommonKeycode::Numpad2, KeycodeMapEntry::NumLock(None, Some('2')));
    keymap.set_common(CommonKeycode::Numpad3, KeycodeMapEntry::NumLock(None, Some('3')));
    keymap.set_common(CommonKeycode::Numpad4, KeycodeMapEntry::NumLock(None, Some('4')));
    keymap.set_common(CommonKeycode::Numpad5, KeycodeMapEntry::NumLock(None, Some('5')));
    keymap.set_common(CommonKeycode::Numpad6, KeycodeMapEntry::NumLock(None, Some('6')));
    keymap.set_common(CommonKeycode::Numpad7, KeycodeMapEntry::NumLock(None, Some('7')));
    keymap.set_common(CommonKeycode::Numpad8, KeycodeMapEntry::NumLock(None, Some('8')));
    keymap.set_common(CommonKeycode::Numpad9, KeycodeMapEntry::NumLock(None, Some('9')));

    keymap
};
