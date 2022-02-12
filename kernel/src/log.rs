use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::io::ansi::AnsiColor;
use crate::io::tty::Tty;
use crate::sync::{Future, UninterruptibleSpinlock};

static OUT_TTY: UninterruptibleSpinlock<Vec<Arc<dyn Tty + Send + Sync>>> = UninterruptibleSpinlock::new(vec![]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Critical,
    Error,
    Warning,
    Notice,
    Info,
    Debug
}

impl LogLevel {
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Critical => "CRIT",
            LogLevel::Error => "ERR",
            LogLevel::Warning => "WARN",
            LogLevel::Notice => "NOTICE",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG"
        }
    }

    pub fn color(self) -> AnsiColor {
        match self {
            LogLevel::Critical => AnsiColor::Red,
            LogLevel::Error => AnsiColor::Red,
            LogLevel::Warning => AnsiColor::Yellow,
            LogLevel::Notice => AnsiColor::Cyan,
            LogLevel::Info => AnsiColor::White,
            LogLevel::Debug => AnsiColor::LightGray
        }
    }
}

pub fn init(out: Arc<dyn Tty + Send + Sync>) {
    OUT_TTY.lock().push(out);
}

pub fn log_msg(msg: String) {
    Future::all(OUT_TTY.lock().iter().map(|tty| {
        // SAFETY: Backing memory for msg is kept alive until all writes are completed by moving it into the when_resolved closure
        unsafe { tty.write(msg.as_bytes()).without_val() }
    }))
    .when_resolved(move |_| drop(msg))
}

#[macro_export]
macro_rules! log {
    ($lvl:ident, $module:expr, $msg:expr $(, $($arg:expr),*)?) => {
        let lvl = ::hydroxos_kernel::log::LogLevel::$lvl;
        ::hydroxos_kernel::log::log_msg(::alloc::format!(
            concat!("[\x1b[{}m{}\x1b[97m] {}: ", $msg, "\n"),
            ::hydroxos_kernel::io::ansi::AnsiParserSgrAction::SetFgColor(lvl.color()),
            lvl.name(),
            $module,
            $($($arg),*)?
        ));
    }
}
