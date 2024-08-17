// Many functions here are accessing lists of arguments and I'd prefer to be consistent in how elements are accessed
#![allow(clippy::get_first)]

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt::{self, Write};

use crate::io::dev;
use crate::io::tty::{Tty, TtyCharReader, TtyWriter};
use crate::sched::task::Process;
use crate::util::ArrayDeque;

struct CommandHistory {
    buf: ArrayDeque<String, 64>,
}

fn readline<T: Tty + ?Sized>(r: &mut TtyCharReader<T>, w: &mut TtyWriter<T>, history: &mut CommandHistory) -> Result<String, String> {
    let mut history_pos = history.buf.len();
    let mut history_modified = [const { None }; 65];

    let mut s = String::new();
    let mut i = 0;

    // TODO Implement line wrapping support

    loop {
        match r.next_char() {
            Ok('\n') => {
                if i != s.len() {
                    let _ = write!(w, "\x1b[{}C", s.len() - i);
                }

                if history.buf.is_full() {
                    history.buf.pop_front();
                }

                assert!(history.buf.push_back(s.clone()).is_ok());

                let _ = writeln!(w);
                return Ok(s);
            },
            Ok('\x7f') => {
                if i != 0 {
                    s.remove(i - 1);
                    i -= 1;

                    let _ = write!(w, "\x1b[D");
                    let _ = write!(w, "{}", &s[i..]);
                    let _ = write!(w, " \x1b[{}D", s.len() - i + 1);
                }
            },
            Ok('\x1b') => match r.next_char() {
                Ok('[') => match r.next_char() {
                    Ok('A') => {
                        if history_pos != 0 {
                            if !s.is_empty() {
                                let _ = write!(w, "\x1b[{}D", i);
                                let _ = write!(w, "\x1b[K");
                            }

                            history_modified[history_pos] = Some(s);
                            history_pos -= 1;

                            if let Some(modified) = history_modified[history_pos].take() {
                                s = modified;
                            } else {
                                s = history.buf.get(history_pos).unwrap().clone();
                            }

                            i = s.len();
                            let _ = write!(w, "{}", s);
                        }
                    },
                    Ok('B') => {
                        if history_pos != history.buf.len() {
                            if !s.is_empty() {
                                let _ = write!(w, "\x1b[{}D", i);
                                let _ = write!(w, "\x1b[K");
                            }

                            history_modified[history_pos] = Some(s);
                            history_pos += 1;

                            if let Some(modified) = history_modified[history_pos].take() {
                                s = modified;
                            } else {
                                s = history.buf.get(history_pos).unwrap().clone();
                            }

                            i = s.len();
                            let _ = write!(w, "{}", s);
                        }
                    },
                    Ok('C') => {
                        if i != s.len() {
                            let _ = write!(w, "\x1b[C");
                            i += 1;
                        }
                    },
                    Ok('D') => {
                        if i != 0 {
                            let _ = write!(w, "\x1b[D");
                            i -= 1;
                        }
                    },
                    _ => {},
                },
                _ => {},
            },
            Ok('\x00'..='\x1f') => {},
            Ok(ch) => {
                let mut ch_bytes = [0_u8; 4];
                let ch_str = ch.encode_utf8(&mut ch_bytes);

                // TODO Add proper UTF-8 support
                if ch_str.len() == 1 {
                    let _ = write!(w, "{}", ch_str);

                    if i != s.len() {
                        let _ = write!(w, "{}", &s[i..]);
                        let _ = write!(w, "\x1b[{}D", s.len() - i);
                    }

                    s.insert(i, ch);
                    i += 1;
                }
            },
            Err(_) => {
                return Err(s);
            },
        }
    }
}

fn run_dev_cmd<T: Tty + ?Sized>(w: &mut TtyWriter<T>, args: &[&str]) -> Result<(), fmt::Error> {
    match args.first() {
        Some(&"ls") => {
            let dev = if let Some(dev_name) = args.get(1) {
                if let Ok(dev) = dev::get_device_by_name(dev_name) {
                    dev
                } else {
                    writeln!(w, "device '{}' was not found", dev_name)?;
                    return Ok(());
                }
            } else {
                dev::device_root().clone()
            };

            dev::print_device_tree(w, &dev)?;
        },
        Some(&"print") => {
            let dev = if let Some(dev_name) = args.get(1) {
                if let Ok(dev) = dev::get_device_by_name(dev_name) {
                    dev
                } else {
                    writeln!(w, "device '{}' was not found", dev_name)?;
                    return Ok(());
                }
            } else {
                dev::device_root().clone()
            };

            // IMPORTANT: Do not print directly to the TTY, since the device we're printing might be involved in the process of printing
            //            text to this TTY and thus could lead to a deadlock.
            let s = format!("{:#?}", dev.dev());
            writeln!(w, "{}", s)?;
        },
        subcmd => {
            if let Some(&subcmd) = subcmd {
                writeln!(w, "unknown dev subcommand '{}'", subcmd)?;
            } else {
                writeln!(w, "no subcommand provided")?;
            }

            writeln!(w, "run 'help dev' for more information")?;
        },
    }

    Ok(())
}

fn run_proc_cmd<T: Tty + ?Sized>(w: &mut TtyWriter<T>, args: &[&str]) -> Result<(), fmt::Error> {
    match args.get(0) {
        Some(&"ls") => {
            for p in &*Process::list() {
                writeln!(w, "{}: {}", p.pid(), p.cmd().get(0).map_or("???", |s| s))?;
            }
        },
        Some(&"threads") => {
            let pid = if let Some(pid) = args.get(1).and_then(|a| a.parse::<u64>().ok()) {
                pid
            } else {
                writeln!(w, "usage: dev threads <pid>")?;
                return Ok(());
            };

            let p = Process::list();
            let p = if let Some(p) = p.get(pid) {
                p
            } else {
                writeln!(w, "no process found with pid {}", pid)?;
                return Ok(());
            };

            for t in p.lock().threads() {
                writeln!(w, "{}: {:?}", t.thread_id(), t.lock().state())?;
            }
        },
        subcmd => {
            if let Some(&subcmd) = subcmd {
                writeln!(w, "unknown proc subcommand '{}'", subcmd)?;
            } else {
                writeln!(w, "no subcommand provided")?;
            }

            writeln!(w, "run 'help proc' for more information")?;
        },
    }

    Ok(())
}

fn run_slab_cmd<T: Tty + ?Sized>(w: &mut TtyWriter<T>, args: &[&str]) -> Result<(), fmt::Error> {
    use crate::mem::slab;

    match args.get(0) {
        None | Some(&"stats") => {
            for alloc in slab::registered_slab_allocs() {
                let (allocated, total) = alloc.lock().count();
                writeln!(w, "{}: {}/{}", alloc.name(), allocated, total)?;
            }
        },
        Some(subcmd) => {
            writeln!(w, "unknown slab subcommand '{}'", subcmd)?;
            writeln!(w, "run 'help slab' for more information")?;
        },
    }

    Ok(())
}

fn run_debug_console_command<T: Tty + ?Sized>(w: &mut TtyWriter<T>, cmd: &[&str]) -> Result<(), fmt::Error> {
    match cmd[0] {
        "dev" => {
            run_dev_cmd(w, &cmd[1..])?;
        },
        "proc" => {
            run_proc_cmd(w, &cmd[1..])?;
        },
        "slab" => {
            run_slab_cmd(w, &cmd[1..])?;
        },
        "help" => match cmd.get(1) {
            None => {
                writeln!(w, "available commands are:")?;
                writeln!(w, "  dev - device information")?;
                writeln!(w, "  proc - process information")?;
                writeln!(w, "  slab - slab alloc statistics")?;
                writeln!(w)?;
                writeln!(w, "run 'help <cmd>' for more information")?;
            },
            Some(&"dev") => {
                writeln!(w, "available subcommands are:")?;
                writeln!(w, "  dev ls [dev] - list devices")?;
                writeln!(w, "  dev print [dev] - print device")?;
            },
            Some(&"proc") => {
                writeln!(w, "available subcommands are:")?;
                writeln!(w, "  proc ls - list processes")?;
                writeln!(w, "  proc threads <pid> - list threads in process")?;
            },
            Some(&"slab") => {
                writeln!(w, "available subcommands are:")?;
                writeln!(w, "  slab stats - print slab allocator statistics")?;
            },
            Some(cmd) => {
                writeln!(w, "unknown command '{}'", cmd)?;
            },
        },
        _ => {
            writeln!(w, "unknown command '{}'", cmd[0])?;
        },
    }

    Ok(())
}

fn parse_command(mut cmd: &str) -> Result<Vec<&str>, (usize, &'static str)> {
    let mut result = vec![];
    let mut idx = 0;

    while !cmd.is_empty() {
        while cmd.starts_with(' ') {
            cmd = &cmd[1..];
            idx += 1;
        }

        if cmd.starts_with('"') {
            // TODO Escape sequences?
            if let Some(quote_len) = cmd[1..].find('"') {
                result.push(&cmd[1..quote_len + 1]);
                cmd = &cmd[quote_len + 2..];
                idx += quote_len + 2;
            } else {
                return Err((idx, "unterminated quoted argument"));
            }
        } else if let Some((cmd_a, cmd_b)) = cmd.split_once(' ') {
            result.push(cmd_a);
            cmd = cmd_b;
            idx += cmd_a.len() + 1;
        } else {
            result.push(cmd);
            cmd = &cmd[cmd.len()..];
        }
    }

    if result.is_empty() {
        result.push("");
    }

    Ok(result)
}

pub fn show_debug_console<T: Tty + ?Sized>(tty: &T) {
    let mut r = TtyCharReader::new(tty);
    let mut w = TtyWriter::new(tty);

    let mut history = CommandHistory { buf: ArrayDeque::new() };

    loop {
        let _ = write!(w, "hkd> ");
        let cmd = readline(&mut r, &mut w, &mut history);

        if let Ok(cmd) = cmd {
            match parse_command(&cmd) {
                Ok(parsed_cmd) => {
                    let _ = run_debug_console_command(&mut w, &parsed_cmd);
                },
                Err((_, msg)) => {
                    let _ = writeln!(w, "parse error: {}", msg);
                },
            }
        } else {
            let _ = writeln!(w);
            let _ = writeln!(w, "io error, exiting hkd");
            break;
        }
    }
}
