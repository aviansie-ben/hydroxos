use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr;

use crate::io::ansi::AnsiColor;
use crate::io::dev::DeviceRef;
use crate::io::tty::Tty;
use crate::options::{self, InvalidOptionValue, KernelOptionParseable};
use crate::sched::enqueue_soft_interrupt;
use crate::sync::{Future, UninterruptibleSpinlock};
use crate::util::OneShotManualInit;

static OUT_TTY: UninterruptibleSpinlock<Vec<DeviceRef<dyn Tty>>> = UninterruptibleSpinlock::new(vec![]);
static LOG_LEVELS: OneShotManualInit<LogLevelOptions> = OneShotManualInit::uninit();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Critical,
    Error,
    Warning,
    Notice,
    Info,
    Debug,
}

impl LogLevel {
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Critical => "CRIT",
            LogLevel::Error => "ERR",
            LogLevel::Warning => "WARN",
            LogLevel::Notice => "NOTICE",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        }
    }

    pub fn color(self) -> AnsiColor {
        match self {
            LogLevel::Critical => AnsiColor::Red,
            LogLevel::Error => AnsiColor::Red,
            LogLevel::Warning => AnsiColor::Yellow,
            LogLevel::Notice => AnsiColor::Cyan,
            LogLevel::Info => AnsiColor::White,
            LogLevel::Debug => AnsiColor::LightGray,
        }
    }
}

impl<'a> KernelOptionParseable<'a> for LogLevel {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        match s {
            "crit" => Ok(LogLevel::Critical),
            "err" => Ok(LogLevel::Error),
            "warn" => Ok(LogLevel::Warning),
            "notice" => Ok(LogLevel::Notice),
            "info" => Ok(LogLevel::Info),
            "debug" => Ok(LogLevel::Debug),
            _ => Err(InvalidOptionValue),
        }
    }
}

struct LogLevelOptions {
    default_level: LogLevel,
    levels: BTreeMap<&'static str, LogLevel>,
}

impl LogLevelOptions {
    #[inline(always)]
    fn use_fast_path(&self) -> bool {
        self.levels.len() == 0
    }
}

pub fn init() {
    let default_level = options::get().get("loglevel").unwrap_or(LogLevel::Info);
    let levels: BTreeMap<_, _> = options::get()
        .iter_group("loglevel")
        .filter_map(|(k, v)| if let Some(v) = v { Some((k, v)) } else { None })
        .collect();

    LOG_LEVELS.set(LogLevelOptions { default_level, levels });
}

pub fn add_tty(out: DeviceRef<dyn Tty>) {
    OUT_TTY.lock().push(out);
}

pub fn remove_tty(out: &DeviceRef<dyn Tty>) -> bool {
    let mut out_tty = OUT_TTY.lock();

    let old_len = out_tty.len();
    out_tty.retain(|tty| !ptr::eq(tty.dev() as *const _ as *const (), out.dev() as *const _ as *const ()));

    out_tty.len() != old_len
}

pub fn log_msg(msg: String) {
    enqueue_soft_interrupt(move || {
        Future::all(OUT_TTY.lock().iter().map(|tty| {
            // SAFETY: Backing memory for msg is kept alive until all writes are completed by moving it into the when_resolved closure
            unsafe { tty.dev().write(msg.as_bytes()).without_val() }
        }))
        .when_resolved(move |_| drop(msg))
    });
}

#[cold]
#[inline(never)]
fn should_log_slow(levels: &LogLevelOptions, lvl: LogLevel, module: &'static str) -> bool {
    lvl <= levels.levels.get(module).copied().unwrap_or(levels.default_level)
}

#[inline(always)]
pub fn should_log(lvl: LogLevel, module: &'static str) -> bool {
    let levels = LOG_LEVELS.get();

    if levels.use_fast_path() {
        lvl <= levels.default_level
    } else {
        should_log_slow(levels, lvl, module)
    }
}

#[macro_export]
macro_rules! log {
    ($lvl:ident, $module:expr, $msg:expr $(, $($arg:expr),*)?) => {
        let lvl = $crate::log::LogLevel::$lvl;
        let module = $module;

        if $crate::log::should_log(lvl, module) {
            $crate::log::log_msg(::alloc::format!(
                concat!("[\x1b[{}m{}\x1b[0m] {}: ", $msg, "\n"),
                $crate::io::ansi::AnsiParserSgrAction::SetFgColor(lvl.color()),
                lvl.name(),
                module,
                $($($arg),*)?
            ));
        }
    }
}

#[macro_export]
macro_rules! dbg {
    () => {
        $crate::log!(Info, "dbg", "[{}:{}:{}]", file!(), line!(), column!());
    };
    ($val:expr $(,)?) => {
        match $val {
            tmp => {
                $crate::log!(
                    Info, "dbg", "[{}:{}:{}] {} = {:?}",
                    file!(), line!(), column!(), stringify!($val), &tmp
                );
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}
