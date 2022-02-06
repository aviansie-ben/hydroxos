#[derive(Debug)]
enum AnsiParserState {
    Normal,
    PartialUtf8(usize, usize),
    Escape,
    PartialCsi(usize)
}

#[derive(Debug)]
pub enum AnsiParserAction {
    WriteChar(char)
}

#[derive(Debug)]
pub struct AnsiParser {
    state: AnsiParserState,
    partial_buf: [u8; AnsiParser::MAX_CSI_LENGTH]
}

impl AnsiParser {
    pub const MAX_CSI_LENGTH: usize = 64;

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
                    b'@'..b'~' => {
                        // TODO Execute CSI
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
