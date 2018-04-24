use std::fmt;

#[derive(PartialEq, PartialOrd)]
pub enum LogLevel {
    Error = 0,
    Info,
    Debug,
    Trace,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            LogLevel::Error => write!(f, "E"),
            LogLevel::Info => write!(f, "I"),
            LogLevel::Debug => write!(f, "D"),
            LogLevel::Trace => write!(f, "T"),
        }
    }
}

pub static mut LEVEL: LogLevel = LogLevel::Info;

pub fn log_init(level: LogLevel) {
    unsafe {
        LEVEL = level;
    }
}

#[macro_export]
macro_rules! trace(
    ($fmt:expr) => {
        if cfg!(debug_assertions) {
            log!($crate::LogLevel::Trace, $fmt)
        }
    };
    ($fmt:expr, $($arg:tt)*) => {
        if cfg!(debug_assertions) {
            log!($crate::LogLevel::Trace, $fmt, $($arg)*)
        }
    };
);

#[macro_export]
macro_rules! debug(
    ($fmt:expr) => {
        log!($crate::LogLevel::Debug, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        log!($crate::LogLevel::Debug, $fmt, $($args)*)
    };
);

#[macro_export]
macro_rules! info(
    ($fmt:expr) => {
        log!($crate::LogLevel::Info, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        log!($crate::LogLevel::Info, $fmt, $($arg)*)
    };
);

#[macro_export]
macro_rules! error(
    ($fmt:expr) => {
        log!($crate::LogLevel::Error, $fmt)
    };
    ($fmt:expr, $($args:tt)*) => {
        log!($crate::LogLevel::Error, $fmt, $($args)*)
    };
);

#[macro_export]
macro_rules! log(
    ($level:expr, $fmt:expr) => {
        {
        #[allow(unused_imports)]
        {
            use std::io::Write;
            use chrono::Local;
            if unsafe { $level <= $crate::log::LEVEL } {
                let mut msg = Vec::with_capacity(25);
                let time = Local::now();
                write!(&mut msg, "{} [{}:{}] {}: ",
                       time.format("%x %X"), module_path!(), line!(), $level).ok();
                write!(&mut msg, $fmt).ok();
                write!(&mut msg, "\n").ok();
                let stderr = ::std::io::stderr();
                let mut handle = stderr.lock();
                handle.write_all(&msg).ok();
            }
        }
        }
    };

    ($level:expr, $fmt:expr, $($arg:tt)*) => {
        {
        #[allow(unused_imports)]
        {
            use std::io::Write;
            use chrono::Local;
            if unsafe { $level <= $crate::log::LEVEL } {
                let mut msg = Vec::with_capacity(25);
                let time = Local::now();
                write!(&mut msg, "{} [{}:{}] {}: ",
                       time.format("%x %X"), module_path!(), line!(), $level).ok();
                write!(&mut msg, $fmt, $($arg)*).ok();
                write!(&mut msg, "\n").ok();
                let stderr = ::std::io::stderr();
                let mut handle = stderr.lock();
                handle.write_all(&msg).ok();
            }
        }
        }
    };
);
