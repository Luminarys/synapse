#![cfg_attr(
    feature = "allocator",
    feature(alloc_system, global_allocator, allocator_api)
)]
#[cfg(feature = "allocator")]
extern crate alloc_system;
#[cfg(feature = "allocator")]
use alloc_system::System;
#[cfg(feature = "allocator")]
#[global_allocator]
static A: System = System;

#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_derive;

#[cfg(test)]
#[macro_use]
extern crate assert_matches;

use synapse_bencode as bencode;
use synapse_protocol as protocol;
use synapse_rpc as rpc_lib;
use synapse_session as session;

#[macro_use]
mod log;
#[macro_use]
mod util;
mod args;
mod buffers;
mod config;
mod control;
mod disk;
mod handle;
mod init;
mod rpc;
mod socket;
mod stat;
mod throttle;
mod torrent;
mod tracker;

use ip_network_table::IpNetworkTable;
use std::process;
use std::sync::atomic;

pub use crate::protocol::DHT_EXT;
pub use crate::protocol::EXT_PROTO;
pub use crate::protocol::UT_META_ID;
pub use crate::protocol::UT_PEX_ID;

/// Throttler max token amount
pub const THROT_TOKS: usize = 2 * 1024 * 1024;

pub static SHUTDOWN: atomic::AtomicBool = atomic::AtomicBool::new(false);

lazy_static! {
    pub static ref CONFIG: config::Config = config::Config::load();
    pub static ref PEER_ID: [u8; 20] = {
        use rand::Rng;

        let mut pid = [0u8; 20];
        let prefix = b"-SY0010-";
        pid[..prefix.len()].clone_from_slice(&prefix[..]);

        let mut rng = rand::thread_rng();
        for p in pid.iter_mut().skip(prefix.len()) {
            *p = rng.gen();
        }
        pid
    };
    pub static ref DL_TOKEN: String = util::random_string(20);
    pub static ref IP_FILTER: IpNetworkTable<u8> = {
        let mut table = IpNetworkTable::new();

        for k in CONFIG.ip_filter.keys() {
            table.insert(k.clone(), CONFIG.ip_filter[k]);
            debug!(
                "Add ip_filter entry: prefix={}, weight={}",
                k, CONFIG.ip_filter[k]
            );
        }
        table
    };
}

fn main() {
    let args = args::args();
    match init::init(args) {
        Ok(()) => {}
        Err(()) => {
            error!("Failed to initialize synapse!");
            process::exit(1);
        }
    }
    info!("Initialized, starting!");
    match init::run() {
        Ok(()) => process::exit(0),
        Err(()) => process::exit(1),
    }
}
