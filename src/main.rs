#![allow(unknown_lints)]
#![allow(unused_doc_comments)]
#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "allocator", feature(alloc_system, global_allocator, allocator_api))]
#[cfg(feature = "allocator")]
extern crate alloc_system;
#[cfg(feature = "allocator")]
use alloc_system::System;
#[cfg(feature = "allocator")]
#[global_allocator]
static A: System = System;

extern crate amy;
extern crate base32;
extern crate base64;
extern crate bincode;
extern crate byteorder;
extern crate chrono;
#[macro_use]
extern crate error_chain;
extern crate fnv;
extern crate fs_extra;
extern crate getopts;
extern crate http_range;
extern crate httparse;
#[macro_use]
extern crate lazy_static;
extern crate memmap;
extern crate metrohash;
extern crate net2;
extern crate nix;
extern crate num_bigint;
extern crate openssl;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate shellexpand;
extern crate toml;
extern crate url;
extern crate vecio;

extern crate adns;
extern crate synapse_bencode as bencode;
extern crate synapse_rpc as rpc_lib;
extern crate synapse_session as session;

#[macro_use]
mod log;
mod args;
mod buffers;
mod handle;
mod torrent;
mod util;
mod socket;
mod disk;
mod tracker;
mod control;
mod listener;
mod rpc;
mod throttle;
mod config;
mod stat;
mod init;

// We need to do this for the log macros
use log::LogLevel;
use std::sync::atomic;
use std::process;

pub const DHT_EXT: (usize, u8) = (7, 1);
pub const EXT_PROTO: (usize, u8) = (5, 0x10);
pub const UT_META_ID: u8 = 9;
pub const UT_PEX_ID: u8 = 11;

/// Throttler max token amount
pub const THROT_TOKS: usize = 2 * 1024 * 1024;

pub static SHUTDOWN: atomic::AtomicBool = atomic::AtomicBool::new(false);

lazy_static! {
    pub static ref CONFIG: config::Config = { config::Config::load() };
    pub static ref PEER_ID: [u8; 20] = {
        use rand::{self, Rng};

        let mut pid = [0u8; 20];
        let prefix = b"-SY0010-";
        for i in 0..prefix.len() {
            pid[i] = prefix[i];
        }

        let mut rng = rand::thread_rng();
        for i in prefix.len()..20 {
            pid[i] = rng.gen::<u8>();
        }
        pid
    };
    pub static ref DL_TOKEN: String = { util::random_string(20) };
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
