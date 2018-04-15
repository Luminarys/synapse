use std::{fs, process};
use std::io::Read;
use std::collections::HashMap;

use toml;
use shellexpand;

pub type Config = HashMap<String, ServerInfo>;

#[derive(Deserialize)]
pub struct ServerInfo {
    pub server: String,
    pub password: String,
}

pub fn load() -> Config {
    enum EK {
        Nonext,
        IO,
        Fmt,
    }

    let files = [
        "./sycli.toml",
        "$XDG_CONFIG_HOME/sycli.toml",
        "~/.config/sycli.toml",
    ];
    for file in &files {
        let mut s = String::new();
        let res = shellexpand::full(&file)
            .map_err(|_| EK::Nonext)
            .and_then(|p| fs::File::open(&*p).map_err(|_| EK::Nonext))
            .and_then(|mut f| f.read_to_string(&mut s).map_err(|_| EK::IO))
            .and_then(|_| toml::from_str(&s).map_err(|_| EK::Fmt));
        match res {
            Ok(cfg) => return cfg,
            Err(EK::Fmt) => {
                eprintln!("Failed to parse config {}, terminating", file,);
                process::exit(1);
            }
            Err(EK::IO) => {
                eprintln!("Failed to load {}, IO error!", file);
            }
            Err(EK::Nonext) => {}
        }
    }
    default()
}

pub fn default() -> Config {
    let mut config = HashMap::with_capacity(1);
    config.insert(
        "default".to_owned(),
        ServerInfo {
            server: "ws://localhost:8412".to_owned(),
            password: "hackme".to_owned(),
        },
    );
    config
}
