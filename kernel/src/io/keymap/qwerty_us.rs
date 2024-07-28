use super::{CommonKeycode, KeyAction, KeycodeMap, KeycodeMapEntry};

pub static KEYMAP: KeycodeMap = {
    use KeyAction::{Char, None, Str};

    let mut keymap = KeycodeMap::new("qwerty-us");

    keymap.set_common(CommonKeycode::Tilde, KeycodeMapEntry::Shift(Char('`'), Char('~')));
    keymap.set_common(CommonKeycode::Num1, KeycodeMapEntry::Shift(Char('1'), Char('!')));
    keymap.set_common(CommonKeycode::Num2, KeycodeMapEntry::Shift(Char('2'), Char('@')));
    keymap.set_common(CommonKeycode::Num3, KeycodeMapEntry::Shift(Char('3'), Char('#')));
    keymap.set_common(CommonKeycode::Num4, KeycodeMapEntry::Shift(Char('4'), Char('$')));
    keymap.set_common(CommonKeycode::Num5, KeycodeMapEntry::Shift(Char('5'), Char('%')));
    keymap.set_common(CommonKeycode::Num6, KeycodeMapEntry::Shift(Char('6'), Char('^')));
    keymap.set_common(CommonKeycode::Num7, KeycodeMapEntry::Shift(Char('7'), Char('&')));
    keymap.set_common(CommonKeycode::Num8, KeycodeMapEntry::Shift(Char('8'), Char('*')));
    keymap.set_common(CommonKeycode::Num9, KeycodeMapEntry::Shift(Char('9'), Char('(')));
    keymap.set_common(CommonKeycode::Num0, KeycodeMapEntry::Shift(Char('0'), Char(')')));
    keymap.set_common(CommonKeycode::Minus, KeycodeMapEntry::Shift(Char('-'), Char('_')));
    keymap.set_common(CommonKeycode::Equal, KeycodeMapEntry::Shift(Char('='), Char('+')));
    keymap.set_common(CommonKeycode::Backspace, KeycodeMapEntry::Simple(Char('\x7f')));

    keymap.set_common(CommonKeycode::LeftBracket, KeycodeMapEntry::Shift(Char('['), Char('{')));
    keymap.set_common(CommonKeycode::RightBracket, KeycodeMapEntry::Shift(Char(']'), Char('}')));
    keymap.set_common(CommonKeycode::Backslash, KeycodeMapEntry::Shift(Char('\\'), Char('|')));
    keymap.set_common(CommonKeycode::Colon, KeycodeMapEntry::Shift(Char(';'), Char(':')));
    keymap.set_common(CommonKeycode::Quote, KeycodeMapEntry::Shift(Char('\''), Char('"')));
    keymap.set_common(CommonKeycode::Enter, KeycodeMapEntry::Simple(Char('\n')));
    keymap.set_common(CommonKeycode::Comma, KeycodeMapEntry::Shift(Char(','), Char('<')));
    keymap.set_common(CommonKeycode::Period, KeycodeMapEntry::Shift(Char('.'), Char('>')));
    keymap.set_common(CommonKeycode::Slash, KeycodeMapEntry::Shift(Char('/'), Char('?')));
    keymap.set_common(CommonKeycode::Space, KeycodeMapEntry::Simple(Char(' ')));

    keymap.set_common(CommonKeycode::A, KeycodeMapEntry::ShiftCaps(Char('a'), Char('A')));
    keymap.set_common(CommonKeycode::B, KeycodeMapEntry::ShiftCaps(Char('b'), Char('B')));
    keymap.set_common(CommonKeycode::C, KeycodeMapEntry::ShiftCaps(Char('c'), Char('C')));
    keymap.set_common(CommonKeycode::D, KeycodeMapEntry::ShiftCaps(Char('d'), Char('D')));
    keymap.set_common(CommonKeycode::E, KeycodeMapEntry::ShiftCaps(Char('e'), Char('E')));
    keymap.set_common(CommonKeycode::F, KeycodeMapEntry::ShiftCaps(Char('f'), Char('F')));
    keymap.set_common(CommonKeycode::G, KeycodeMapEntry::ShiftCaps(Char('g'), Char('G')));
    keymap.set_common(CommonKeycode::H, KeycodeMapEntry::ShiftCaps(Char('h'), Char('H')));
    keymap.set_common(CommonKeycode::I, KeycodeMapEntry::ShiftCaps(Char('i'), Char('I')));
    keymap.set_common(CommonKeycode::J, KeycodeMapEntry::ShiftCaps(Char('j'), Char('J')));
    keymap.set_common(CommonKeycode::K, KeycodeMapEntry::ShiftCaps(Char('k'), Char('K')));
    keymap.set_common(CommonKeycode::L, KeycodeMapEntry::ShiftCaps(Char('l'), Char('L')));
    keymap.set_common(CommonKeycode::M, KeycodeMapEntry::ShiftCaps(Char('m'), Char('M')));
    keymap.set_common(CommonKeycode::N, KeycodeMapEntry::ShiftCaps(Char('n'), Char('N')));
    keymap.set_common(CommonKeycode::O, KeycodeMapEntry::ShiftCaps(Char('o'), Char('O')));
    keymap.set_common(CommonKeycode::P, KeycodeMapEntry::ShiftCaps(Char('p'), Char('P')));
    keymap.set_common(CommonKeycode::Q, KeycodeMapEntry::ShiftCaps(Char('q'), Char('Q')));
    keymap.set_common(CommonKeycode::R, KeycodeMapEntry::ShiftCaps(Char('r'), Char('R')));
    keymap.set_common(CommonKeycode::S, KeycodeMapEntry::ShiftCaps(Char('s'), Char('S')));
    keymap.set_common(CommonKeycode::T, KeycodeMapEntry::ShiftCaps(Char('t'), Char('T')));
    keymap.set_common(CommonKeycode::U, KeycodeMapEntry::ShiftCaps(Char('u'), Char('U')));
    keymap.set_common(CommonKeycode::V, KeycodeMapEntry::ShiftCaps(Char('v'), Char('V')));
    keymap.set_common(CommonKeycode::W, KeycodeMapEntry::ShiftCaps(Char('w'), Char('W')));
    keymap.set_common(CommonKeycode::X, KeycodeMapEntry::ShiftCaps(Char('x'), Char('X')));
    keymap.set_common(CommonKeycode::Y, KeycodeMapEntry::ShiftCaps(Char('y'), Char('Y')));
    keymap.set_common(CommonKeycode::Z, KeycodeMapEntry::ShiftCaps(Char('z'), Char('Z')));

    keymap.set_common(CommonKeycode::NumpadSlash, KeycodeMapEntry::Simple(Char('/')));
    keymap.set_common(CommonKeycode::NumpadTimes, KeycodeMapEntry::Simple(Char('*')));
    keymap.set_common(CommonKeycode::NumpadMinus, KeycodeMapEntry::Simple(Char('-')));
    keymap.set_common(CommonKeycode::NumpadPlus, KeycodeMapEntry::Simple(Char('+')));
    keymap.set_common(CommonKeycode::NumpadDot, KeycodeMapEntry::NumLock(None, Char('.')));
    keymap.set_common(CommonKeycode::NumpadEnter, KeycodeMapEntry::Simple(Char('\n')));

    keymap.set_common(CommonKeycode::Numpad0, KeycodeMapEntry::NumLock(None, Char('0')));
    keymap.set_common(CommonKeycode::Numpad1, KeycodeMapEntry::NumLock(None, Char('1')));
    keymap.set_common(CommonKeycode::Numpad2, KeycodeMapEntry::NumLock(None, Char('2')));
    keymap.set_common(CommonKeycode::Numpad3, KeycodeMapEntry::NumLock(None, Char('3')));
    keymap.set_common(CommonKeycode::Numpad4, KeycodeMapEntry::NumLock(None, Char('4')));
    keymap.set_common(CommonKeycode::Numpad5, KeycodeMapEntry::NumLock(None, Char('5')));
    keymap.set_common(CommonKeycode::Numpad6, KeycodeMapEntry::NumLock(None, Char('6')));
    keymap.set_common(CommonKeycode::Numpad7, KeycodeMapEntry::NumLock(None, Char('7')));
    keymap.set_common(CommonKeycode::Numpad8, KeycodeMapEntry::NumLock(None, Char('8')));
    keymap.set_common(CommonKeycode::Numpad9, KeycodeMapEntry::NumLock(None, Char('9')));

    keymap.set_common(CommonKeycode::UpArrow, KeycodeMapEntry::Simple(Str("\x1b[A")));
    keymap.set_common(CommonKeycode::DownArrow, KeycodeMapEntry::Simple(Str("\x1b[B")));
    keymap.set_common(CommonKeycode::LeftArrow, KeycodeMapEntry::Simple(Str("\x1b[D")));
    keymap.set_common(CommonKeycode::RightArrow, KeycodeMapEntry::Simple(Str("\x1b[C")));

    keymap
};
