use alloc::{format, vec};
use alloc::{string::String, vec::Vec};
use core::fmt::{self, Write};

use crate::io::dev;
use crate::io::tty::{Tty, TtyCharReader, TtyWriter};
use crate::sched::task::Process;

fn readline<T: Tty + ?Sized>(r: &mut TtyCharReader<T>, w: &mut TtyWriter<T>) -> Result<String, String> {
    let mut s = String::new();

    loop {
        match r.next_char() {
            Ok('\n') => {
                let _ = writeln!(w);
                return Ok(s);
            },
            Ok('\x7f') => {
                if s.pop().is_some() {
                    let _ = write!(w, "\x08 \x08");
                }
            },
            Ok('\x00'..='\x1f') => {},
            Ok(ch) => {
                let mut ch_bytes = [0_u8; 4];
                let _ = write!(w, "{}", ch.encode_utf8(&mut ch_bytes));
                s.push(ch);
            },
            Err(_) => {
                return Err(s);
            }
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
        }
    }

    Ok(())
}

fn run_proc_cmd<T: Tty + ?Sized>(w: &mut TtyWriter<T>, args: &[&str]) -> Result<(), fmt::Error> {
    match args.get(0) {
        Some(&"ls") => {
            for p in &*Process::list() {
                writeln!(w, "{}: {}", p.pid(), p.cmd().get(0).map_or("???", |s| &*s))?;
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
        }
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
        "help" => match cmd.get(1) {
            None => {
                writeln!(w, "available commands are:")?;
                writeln!(w, "  dev - device information")?;
                writeln!(w, "  proc - process information")?;
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
            Some(cmd) => {
                writeln!(w, "unknown command '{}'", cmd)?;
            }
        },
        _ => {
            writeln!(w, "unknown command '{}'", cmd[0])?;
        }
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

    loop {
        let _ = write!(w, "hkd> ");
        let cmd = readline(&mut r, &mut w);

        if let Ok(cmd) = cmd {
            match parse_command(&cmd) {
                Ok(parsed_cmd) => {
                    let _ = run_debug_console_command(&mut w, &parsed_cmd);
                },
                Err((_, msg)) => {
                    let _ = writeln!(w, "parse error: {}", msg);
                }
            }
        } else {
            let _ = writeln!(w);
            let _ = writeln!(w, "io error, exiting hkd");
            break;
        }
    }
}
