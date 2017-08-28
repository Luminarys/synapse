#[derive(PartialEq, PartialOrd)]
pub enum LogLevel {
    Error = 0,
    Info,
    Debug,
    Trace,
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
        log!($crate::LogLevel::Trace, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        log!($crate::LogLevel::Trace, $fmt, $($arg)*)
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
    ($fmt:expr, $($args:tt),*) => {
        log!($crate::LogLevel::Error, $fmt, $($args)*)
    };
);

#[macro_export]
macro_rules! log(
    ($level:expr, $fmt:expr) => {
        {
            use std::io::Write;
            use chrono::Local;
            if unsafe { $level <= $crate::log::LEVEL } {
                let mut msg = Vec::with_capacity(25);
                let time = Local::now();
                write!(&mut msg, "{} - [{}:{}] ", time.format("%x %X"), file!(), line!()).unwrap();
                write!(&mut msg, $fmt).unwrap();
                write!(&mut msg, "\n").unwrap();
                let stderr = ::std::io::stderr();
                let mut handle = stderr.lock();
                handle.write(&msg).unwrap();
            }
        }
    };

    ($level:expr, $fmt:expr, $($arg:tt)*) => {
        {
            use std::io::Write;
            use chrono::Local;
            if unsafe { $level <= $crate::log::LEVEL } {
                let mut msg = Vec::with_capacity(25);
                let time = Local::now();
                write!(&mut msg, "{} - [{}:{}] ", time.format("%x %X"), file!(), line!()).unwrap();
                write!(&mut msg, $fmt, $($arg)*).unwrap();
                write!(&mut msg, "\n").unwrap();
                let stderr = ::std::io::stderr();
                let mut handle = stderr.lock();
                handle.write(&msg).unwrap();
            }
        }
    };
);
