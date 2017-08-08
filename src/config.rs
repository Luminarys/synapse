use std::env;
use std::net::{SocketAddr, ToSocketAddrs};

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub trk_port: u16,
    pub dht_port: u16,
    pub dht_bootstrap_node: Option<SocketAddr>,
    pub rpc_port: u16,
    pub session: String,
    pub directory: String,
}

#[derive(Serialize, Deserialize)]
pub struct ConfigFile {
    pub port: Option<u16>,
    pub rpc_port: Option<u16>,
    pub trk_port: Option<u16>,
    pub dht_port: Option<u16>,
    pub dht_bootstrap_node: Option<String>,
    pub session: Option<String>,
    pub directory: Option<String>,
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
        if let Some(p) = file.trk_port {
            base.trk_port = p
        }
        if let Some(p) = file.dht_port {
            base.dht_port = p
        }
        if let Some(n) = file.dht_bootstrap_node {
            match (&n).to_socket_addrs() {
                Ok(mut a) => base.dht_bootstrap_node = a.next(),
                _ => {}
            }
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

impl Default for Config {
    fn default() -> Config {
        let s = "~/.syn_session".to_owned();
        Config {
            port: 16493,
            rpc_port: 8412,
            trk_port: 16362,
            dht_port: 14831,
            dht_bootstrap_node: None,
            session: expand_tilde(&s),
            directory: "./".to_owned(),
        }
    }
}

fn expand_tilde(s: &str) -> String {
    s.replace(
        '~',
        &env::home_dir()
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap(),
    )
}
