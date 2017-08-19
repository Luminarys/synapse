#[macro_use]
extern crate error_chain;
extern crate clap;
extern crate rpc;
extern crate serde;
extern crate serde_json;
extern crate reqwest;
extern crate url;
extern crate websocket;

use std::process;

use url::Url;
use clap::{App, AppSettings, Arg, SubCommand};
use websocket::ClientBuilder;

mod cmd;

fn main() {
    let matches = App::new("sycli")
        .about("cli interface for synapse")
        .author(env!("CARGO_PKG_AUTHORS"))
        .version(env!("CARGO_PKG_VERSION"))
        .setting(AppSettings::SubcommandRequired)
        .arg(Arg::with_name("server")
             .help("URI of the synapse client to connect to.")
             .short("s")
             .long("server")
             .default_value("ws://localhost:8412/"))
        .arg(Arg::with_name("password")
             .help("Password to use when connecting to synapse.")
             .short("p")
             .long("password")
             .takes_value(true))
        .subcommand(SubCommand::with_name("add")
                    .about("Adds torrents to synapse.")
                    .arg(Arg::with_name("directory")
                         .help("Custom directory to download the torrent to.")
                         .short("d")
                         .long("directory")
                         .takes_value(true))
                    .arg(Arg::with_name("files")
                         .help("Torrent files to add")
                         .multiple(true)
                         .short("f")
                         .long("files")
                         .required(true)
                         .index(1))
                   )
        .subcommand(SubCommand::with_name("del")
                    .about("Deletes torrents from synapse.")
                    .arg(Arg::with_name("torrents")
                         .help("Names of torrents to delete. A fuzzy match will be attempted and ambiguities displayed.")
                         .multiple(true)
                         .short("t")
                         .long("torrents")
                         .required(true)
                         .index(1))
                   )
        .subcommand(SubCommand::with_name("list")
                    .about("Lists torrents in synapse.")
                    .arg(Arg::with_name("active")
                         .help("Only display non idle and pending torrents.")
                         .short("a")
                         .long("active"))
                    .arg(Arg::with_name("output")
                         .help("Output the results in the specified format.")
                         .short("o")
                         .long("output")
                         .possible_values(&["json", "text"])
                         .default_value("text")
                        )
                   )
        .subcommand(SubCommand::with_name("rate")
                    .about("Rate limits synapse.")
                    .arg(Arg::with_name("up")
                         .help("Global upload rate.")
                         .short("u")
                         .long("upload")
                         .index(1))
                    .arg(Arg::with_name("down")
                         .help("Global download rate.")
                         .short("d")
                         .long("download")
                         .index(2))
                   )
        .subcommand(SubCommand::with_name("start")
                    .about("Starts torrents in synapse.")
                    .arg(Arg::with_name("torrents")
                         .help("Names of torrents to start. A fuzzy match will be attempted and ambiguities displayed.")
                         .multiple(true)
                         .short("t")
                         .long("torrents")
                         .index(1))
                   )
        .subcommand(SubCommand::with_name("stop")
                    .about("Stops torrents in synapse.")
                    .arg(Arg::with_name("torrents")
                         .help("Names of torrents to stop. A fuzzy match will be attempted and ambiguities displayed.")
                         .multiple(true)
                         .short("t")
                         .long("torrents")
                         .index(1))
                   )
        .subcommand(SubCommand::with_name("dl")
                    .about("Downloads a torrent.")
                    .arg(Arg::with_name("torrent")
                         .help("Name of torrent to download. A fuzzy match will be attempted and ambiguities displayed.")
                         .short("t")
                         .long("torrent")
                         .index(1))
                   )
        .get_matches();

    let mut url = match Url::parse(matches.value_of("server").unwrap()) {
        Ok(url) => url,
        Err(_) => {
            eprintln!("Couldn't parse server URI!");
            process::exit(1);
        }
    };
    if let Some(password) = matches.value_of("password") {
        url.query_pairs_mut().append_pair("password", password);
    }
    let client = match ClientBuilder::new(url.as_str()).unwrap().connect(None) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Couldn't connect to synapse!");
            process::exit(1);
        }
    };
    if url.scheme() == "wss" {
        url.set_scheme("https").unwrap();
    } else {
        url.set_scheme("http").unwrap();
    }

    match matches.subcommand_name().unwrap() {
        "add" => {
            let args = matches.subcommand_matches("add").unwrap();
            let mut files = Vec::new();
            for file in args.values_of("files").unwrap() {
                files.push(file)
            }
            let res = cmd::add(client, url.as_str(), files, args.value_of("directory"));
            if res.is_err() {
                eprintln!("Failed to add torrents: {:?}", res.err().unwrap());
                process::exit(1);
            }
        }
        "del" => {
            let args = matches.subcommand_matches("del").unwrap();
            let res = cmd::del(client, args.values_of("torrents").unwrap().collect());
            if res.is_err() {
                eprintln!("Failed to delete torrents: {:?}", res.err().unwrap());
                process::exit(1);
            }
        }
        "dl" => {
            cmd::dl(client);
        }
        "list" => {
            cmd::list(client);
        }
        "rate" => {
            cmd::rate(client);
        }
        "start" => {
            cmd::start(client);
        }
        "stop" => {
            cmd::stop(client);
        }
        _ => { },
    }
}
