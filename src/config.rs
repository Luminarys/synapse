use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub rpc_port: u16,
    pub session: String,
    pub directory: String,
}

impl Config {
    pub fn from_file(file: ConfigFile) -> Config {
        let mut base: Config = Default::default();
        if let Some(p) = file.port {
            base.port = p
        }
        if let Some(p) = file.rpc_port {
            base.rpc_port = p
        }
        if let Some(s) = file.session {
            base.session = expand_tilde(&s);
        }
        if let Some(d) = file.directory {
            base.directory = expand_tilde(&d)
        }
        base
    }
}

#[derive(Serialize, Deserialize)]
pub struct ConfigFile {
    pub port: Option<u16>,
    pub rpc_port: Option<u16>,
    pub session: Option<String>,
    pub directory: Option<String>,
}

impl Default for Config {
    fn default() -> Config {
        let s = "~/.syn_session".to_owned();
        ;
        Config {
            port: 16493,
            rpc_port: 8412,
            session: expand_tilde(&s),
            directory: "./".to_owned(),
        }
    }
}

fn expand_tilde(s: &String) -> String {
    s.replace('~', &env::home_dir().unwrap().into_os_string().into_string().unwrap())
}
