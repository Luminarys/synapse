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
            base.session = s
        }
        if let Some(d) = file.directory {
            base.directory = d
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
        Config {
            port: 16493,
            rpc_port: 8412,
            session: "~/.syn_session".to_owned(),
            directory: "./".to_owned(),
        }
    }
}
