use std::env;
use std::process;

use getopts::Options;

use crate::log;

pub struct Args {
    pub config: Option<String>,
    pub level: Option<log::LogLevel>,
}

pub fn args() -> Args {
    let args: Vec<String> = env::args().collect();
    let mut opts = Options::new();
    opts.optflag("h", "help", "Show help message.");
    opts.optflag("d", "debug", "Enable debug logging.");
    opts.optopt("c", "config", "Use config file.", "FILE");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            println!("Failed to parse options: {}", f);
            usage(1, opts);
        }
    };

    if matches.opt_present("h") {
        usage(0, opts);
    }

    let mut args = Args {
        config: None,
        level: None,
    };

    if matches.opt_present("d") {
        args.level = Some(log::LogLevel::Debug);
    }

    if let Some(cfg) = matches.opt_str("c") {
        args.config = Some(cfg);
    }

    args
}

fn usage(code: i32, opts: Options) -> ! {
    let brief = format!("Usage: synapse [options]");
    print!("{}", opts.usage(&brief));
    process::exit(code);
}
