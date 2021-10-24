use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::{fs, process};

use crate::args;

error_chain! {
    errors {
        Env {
            description("bad env var")
                display("bad env var")
        }

        IO {
            description("IO failed")
                display("IO failed")
        }

        Format {
            description("invalid config format")
                display("invalid config format")
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub max_dl: u32,
    pub trk: TrkConfig,
    pub dht: DhtConfig,
    pub rpc: RpcConfig,
    pub disk: DiskConfig,
    pub net: NetConfig,
    pub peer: PeerConfig,
}

#[derive(Debug, Clone)]
pub struct DhtConfig {
    pub port: u16,
    pub bootstrap_node: Option<SocketAddr>,
}

#[derive(Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_dl")]
    pub max_dl: u32,
    #[serde(default)]
    pub rpc: RpcConfig,
    #[serde(default)]
    pub tracker: TrkConfig,
    #[serde(default)]
    pub dht: DhtConfigFile,
    #[serde(default)]
    pub disk: DiskConfig,
    #[serde(default)]
    pub net: NetConfig,
    #[serde(default)]
    pub peer: PeerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    #[serde(default = "default_rpc_port")]
    pub port: u16,
    #[serde(default = "default_local")]
    pub local: bool,
    #[serde(default = "default_auth")]
    pub auth: bool,
    #[serde(default = "default_password")]
    pub password: String,
    #[serde(default = "default_ssl")]
    pub ssl_cert: String,
    #[serde(default = "default_ssl")]
    pub ssl_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrkConfig {
    #[serde(default = "default_trk_port")]
    pub port: u16,
    #[serde(default = "default_verify_certificates")]
    pub verify_certificates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtConfigFile {
    #[serde(default = "default_dht_port")]
    pub port: u16,
    #[serde(default = "default_bootstrap_node")]
    pub bootstrap_node: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConfig {
    #[serde(default = "default_session_dir")]
    pub session: String,
    #[serde(default = "default_directory_dir")]
    pub directory: String,
    #[serde(default = "default_validate")]
    pub validate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    #[serde(default = "default_max_files")]
    pub max_open_files: usize,
    #[serde(default = "default_max_sockets")]
    pub max_open_sockets: usize,
    #[serde(default = "default_max_announces")]
    pub max_open_announces: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    #[serde(default = "default_prune_timeout")]
    pub prune_timeout: u64,
}

impl ConfigFile {
    pub fn try_load() -> Result<ConfigFile> {
        let args = args::args();
        let files = [
            args.config
                .as_ref()
                .map(String::as_str)
                .unwrap_or("./config.toml"),
            "$XDG_CONFIG_HOME/synapse.toml",
            "~/.config/synapse.toml",
        ];
        for file in &files {
            let mut s = String::new();
            let res: Result<ConfigFile> = shellexpand::full(&file)
                .chain_err(|| ErrorKind::Env)
                .and_then(|p| fs::File::open(&*p).chain_err(|| ErrorKind::IO))
                .and_then(|mut f| f.read_to_string(&mut s).chain_err(|| ErrorKind::IO))
                .and_then(|_| toml::from_str(&s).chain_err(|| ErrorKind::Format));
            match res {
                Ok(mut cfg) => {
                    if cfg.max_dl == 0 {
                        error!("Config max_dl must not be 0");
                        process::exit(1);
                    }
                    if !cfg!(debug_assertions) && !cfg.disk.validate {
                        error!("validation skipping can only be used in development, overriding!");
                        cfg.disk.validate = true;
                    }
                    return Ok(cfg);
                }
                Err(e @ Error(ErrorKind::Format, _)) => {
                    use std::error::Error;
                    error!(
                        "Failed to parse config, terminating: {}",
                        e.source().unwrap()
                    );
                    process::exit(1);
                }
                Err(e) => {
                    debug!("Failed to load config file {}: {}", file, e);
                }
            }
        }
        bail!("Failed to find a suitable config!");
    }
}

impl Config {
    pub fn load() -> Config {
        if let Ok(cfg) = ConfigFile::try_load() {
            info!("Loaded config file");
            Config::from_file(cfg)
        } else {
            info!("Using default config");
            Default::default()
        }
    }

    pub fn from_file(mut file: ConfigFile) -> Config {
        let addr = file
            .dht
            .bootstrap_node
            .and_then(|n| n.to_socket_addrs().ok())
            .and_then(|mut a| a.next());
        let dht = DhtConfig {
            port: file.dht.port,
            bootstrap_node: addr,
        };
        file.disk.session = shellexpand::tilde(&file.disk.session).into();
        file.disk.directory = shellexpand::tilde(&file.disk.directory).into();
        Config {
            port: file.port,
            max_dl: file.max_dl,
            trk: file.tracker,
            rpc: file.rpc,
            disk: file.disk,
            net: file.net,
            peer: file.peer,
            dht,
        }
    }
}

fn default_port() -> u16 {
    16_384
}
fn default_max_dl() -> u32 {
    10
}
fn default_trk_port() -> u16 {
    16_362
}
fn default_dht_port() -> u16 {
    16_309
}
fn default_rpc_port() -> u16 {
    8_412
}
fn default_local() -> bool {
    true
}
fn default_auth() -> bool {
    false
}
fn default_password() -> String {
    "hackme".to_owned()
}
fn default_ssl() -> String {
    "".to_owned()
}
fn default_bootstrap_node() -> Option<String> {
    None
}
fn default_session_dir() -> String {
    shellexpand::full("$XDG_DATA_HOME/synapse")
        .unwrap_or_else(|_| shellexpand::tilde("~/.local/share/synapse"))
        .into()
}
fn default_directory_dir() -> String {
    "./".into()
}
fn default_validate() -> bool {
    true
}
fn default_max_files() -> usize {
    500
}
fn default_max_sockets() -> usize {
    400
}
fn default_max_announces() -> usize {
    50
}
fn default_prune_timeout() -> u64 {
    15
}

fn default_verify_certificates() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: default_port(),
            max_dl: default_max_dl(),
            trk: Default::default(),
            rpc: Default::default(),
            disk: Default::default(),
            net: Default::default(),
            dht: Default::default(),
            peer: Default::default(),
        }
    }
}

impl Default for RpcConfig {
    fn default() -> RpcConfig {
        RpcConfig {
            port: default_rpc_port(),
            local: default_local(),
            auth: default_auth(),
            password: default_password(),
            ssl_cert: default_ssl(),
            ssl_key: default_ssl(),
        }
    }
}

impl Default for TrkConfig {
    fn default() -> TrkConfig {
        TrkConfig {
            port: default_trk_port(),
            verify_certificates: default_verify_certificates(),
        }
    }
}

impl Default for DhtConfigFile {
    fn default() -> DhtConfigFile {
        DhtConfigFile {
            port: default_dht_port(),
            bootstrap_node: default_bootstrap_node(),
        }
    }
}

impl Default for DhtConfig {
    fn default() -> DhtConfig {
        DhtConfig {
            port: default_dht_port(),
            bootstrap_node: None,
        }
    }
}

impl Default for DiskConfig {
    fn default() -> DiskConfig {
        DiskConfig {
            session: default_session_dir(),
            directory: default_directory_dir(),
            validate: default_validate(),
        }
    }
}

impl Default for NetConfig {
    fn default() -> NetConfig {
        NetConfig {
            max_open_files: default_max_files(),
            max_open_sockets: default_max_sockets(),
            max_open_announces: default_max_announces(),
        }
    }
}

impl Default for PeerConfig {
    fn default() -> PeerConfig {
        PeerConfig {
            prune_timeout: default_prune_timeout(),
        }
    }
}
