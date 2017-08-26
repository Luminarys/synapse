use std::env;
use std::net::{SocketAddr, ToSocketAddrs};

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrkConfig {
    #[serde(default = "default_trk_port")]
    pub port: u16,
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

impl Config {
    pub fn from_file(mut file: ConfigFile) -> Config {
        let addr = file.dht
            .bootstrap_node
            .and_then(|n| n.to_socket_addrs().ok())
            .and_then(|mut a| a.next());
        let dht = DhtConfig {
            port: file.dht.port,
            bootstrap_node: addr,
        };
        file.disk.session = expand_tilde(&file.disk.session);
        file.disk.directory = expand_tilde(&file.disk.directory);
        Config {
            port: file.port,
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
fn default_bootstrap_node() -> Option<String> {
    None
}
fn default_session_dir() -> String {
    expand_tilde("~/.syn_session")
}
fn default_directory_dir() -> String {
    expand_tilde("./")
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

impl Default for Config {
    fn default() -> Self {
        Config {
            port: default_port(),
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
        }
    }
}

impl Default for TrkConfig {
    fn default() -> TrkConfig {
        TrkConfig { port: default_trk_port() }
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
        PeerConfig { prune_timeout: default_prune_timeout() }
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
