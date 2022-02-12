use core::fmt;

#[derive(Debug)]
enum AnsiParserState {
    Normal,
    PartialUtf8(usize, usize),
    Escape,
    PartialCsi(usize)
}

#[derive(Debug, Clone, Copy)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Brown,
    Blue,
    Magenta,
    Cyan,
    LightGray,
    DarkGray,
    LightRed,
    LightGreen,
    Yellow,
    LightBlue,
    Pink,
    LightCyan,
    White
}

impl AnsiColor {
    fn code_offset(self) -> u32 {
        match self {
            AnsiColor::Black => 0,
            AnsiColor::Red => 1,
            AnsiColor::Green => 2,
            AnsiColor::Brown => 3,
            AnsiColor::Blue => 4,
            AnsiColor::Magenta => 5,
            AnsiColor::Cyan => 6,
            AnsiColor::LightGray => 7,
            AnsiColor::DarkGray => 60,
            AnsiColor::LightRed => 61,
            AnsiColor::LightGreen => 62,
            AnsiColor::Yellow => 63,
            AnsiColor::LightBlue => 64,
            AnsiColor::Pink => 65,
            AnsiColor::LightCyan => 66,
            AnsiColor::White => 67
        }
    }

    fn from_code_offset(code_off: u32) -> Option<AnsiColor> {
        Some(match code_off {
            0 => AnsiColor::Black,
            1 => AnsiColor::Red,
            2 => AnsiColor::Green,
            3 => AnsiColor::Brown,
            4 => AnsiColor::Blue,
            5 => AnsiColor::Magenta,
            6 => AnsiColor::Cyan,
            7 => AnsiColor::LightGray,
            60 => AnsiColor::DarkGray,
            61 => AnsiColor::LightRed,
            62 => AnsiColor::LightGreen,
            63 => AnsiColor::Yellow,
            64 => AnsiColor::LightBlue,
            65 => AnsiColor::Pink,
            66 => AnsiColor::LightCyan,
            67 => AnsiColor::White,
            _ => {
                return None;
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AnsiParserSgrAction {
    Reset,
    SetFgColor(AnsiColor),
    SetBgColor(AnsiColor)
}

impl fmt::Display for AnsiParserSgrAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            AnsiParserSgrAction::Reset => write!(f, "0"),
            AnsiParserSgrAction::SetFgColor(color) => write!(f, "{}", 30 + color.code_offset()),
            AnsiParserSgrAction::SetBgColor(color) => write!(f, "{}", 40 + color.code_offset())
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AnsiParserAction {
    WriteChar(char),
    Sgr([AnsiParserSgrAction; AnsiParser::MAX_SGR_CMDS], usize)
}

#[derive(Debug)]
pub struct AnsiParser {
    state: AnsiParserState,
    partial_buf: [u8; AnsiParser::MAX_CSI_LENGTH]
}

fn parse_ansi_sgr(sgr: &[u8]) -> ([AnsiParserSgrAction; AnsiParser::MAX_SGR_CMDS], usize) {
    let mut cmds = [0; AnsiParser::MAX_SGR_CMDS];
    let mut cmds_len = 0;

    let mut current_cmd: u32 = 0;

    for ch in sgr {
        match ch {
            b';' => {
                if cmds_len < cmds.len() {
                    cmds[cmds_len] = current_cmd;
                    cmds_len += 1;
                }

                current_cmd = 0;
            },
            b'0'..=b'9' => {
                current_cmd = current_cmd.saturating_mul(10).saturating_add((ch - b'0') as u32);
            },
            _ => {}
        }
    }

    if cmds_len < cmds.len() {
        cmds[cmds_len] = current_cmd;
        cmds_len += 1;
    }

    let mut actions = [AnsiParserSgrAction::Reset; AnsiParser::MAX_SGR_CMDS];
    let mut actions_len = 0;
    let mut cmds_idx = 0;

    while cmds_idx < cmds_len && actions_len < actions.len() {
        let cmd = cmds[cmds_idx];
        cmds_idx += 1;

        let action = match cmd {
            0 => Some(AnsiParserSgrAction::Reset),
            30..=37 | 90..=97 => Some(AnsiParserSgrAction::SetFgColor(AnsiColor::from_code_offset(cmd - 30).unwrap())),
            40..=47 | 100..=107 => Some(AnsiParserSgrAction::SetBgColor(AnsiColor::from_code_offset(cmd - 40).unwrap())),
            _ => None
        };

        if let Some(action) = action {
            actions[actions_len] = action;
            actions_len += 1;
        }
    }

    (actions, actions_len)
}

impl AnsiParser {
    pub const MAX_CSI_LENGTH: usize = 64;
    pub const MAX_SGR_CMDS: usize = 8;

    pub fn new() -> AnsiParser {
        AnsiParser { state: AnsiParserState::Normal, partial_buf: [0; AnsiParser::MAX_CSI_LENGTH] }
    }

    pub fn reset(&mut self) {
        self.state = AnsiParserState::Normal;
    }

    pub fn write(&mut self, b: u8) -> Option<AnsiParserAction> {
        match self.state {
            AnsiParserState::Normal => match b {
                b'\x1b' => {
                    self.state = AnsiParserState::Escape;
                    None
                },
                b'\xc0'..=b'\xdf' => {
                    self.partial_buf[0] = b;
                    self.state = AnsiParserState::PartialUtf8(1, 2);
                    None
                },
                b'\xe0'..=b'\xef' => {
                    self.partial_buf[0] = b;
                    self.state = AnsiParserState::PartialUtf8(1, 3);
                    None
                },
                b'\xf0'..=b'\xff' => {
                    self.partial_buf[0] = b;
                    self.state = AnsiParserState::PartialUtf8(1, 4);
                    None
                },
                _ => {
                    Some(AnsiParserAction::WriteChar(b as char))
                }
            },
            AnsiParserState::PartialUtf8(i, len) => {
                self.partial_buf[i] = b;

                if i + 1 == len {
                    self.state = AnsiParserState::Normal;
                    Some(AnsiParserAction::WriteChar(if let Ok(s) = core::str::from_utf8(&self.partial_buf[0..len]) {
                        s.chars().next().unwrap()
                    } else {
                        '\u{fffd}'
                    }))
                } else {
                    self.state = AnsiParserState::PartialUtf8(i + 1, len);
                    None
                }
            },
            AnsiParserState::Escape => match b {
                b'[' => {
                    self.state = AnsiParserState::PartialCsi(0);
                    None
                },
                _ => {
                    self.state = AnsiParserState::Normal;
                    None
                }
            },
            AnsiParserState::PartialCsi(AnsiParser::MAX_CSI_LENGTH) => match b {
                b'@'..b'~' => {
                    self.state = AnsiParserState::Normal;
                    None
                },
                _ => None
            },
            AnsiParserState::PartialCsi(i) => {
                self.partial_buf[i] = b;

                match b {
                    b'm' => {
                        self.state = AnsiParserState::Normal;

                        let (sgr, sgr_len) = parse_ansi_sgr(&self.partial_buf[0..i]);
                        Some(AnsiParserAction::Sgr(sgr, sgr_len))
                    },
                    b'@'..b'~' => {
                        self.state = AnsiParserState::Normal;
                        None
                    },
                    _ => {
                        self.state = AnsiParserState::PartialCsi(i + 1);
                        None
                    }
                }
            }
        }
    }
}
