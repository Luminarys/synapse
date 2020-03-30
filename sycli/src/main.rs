#![allow(unused_doc_comments)]

#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate prettytable;
#[macro_use]
extern crate serde_derive;
extern crate synapse_rpc as rpc;
extern crate tungstenite as ws;

mod client;
mod cmd;
mod config;
mod error;

use std::process;

use clap::{App, AppSettings, Arg, SubCommand};
use error_chain::ChainedError;
use url::Url;

use self::client::Client;

fn main() {
    let config = config::load();
    let matches = App::new("sycli")
        .about("cli interface for synapse")
        .author(env!("CARGO_PKG_AUTHORS"))
        .version(env!("CARGO_PKG_VERSION"))
        .global_setting(AppSettings::ColoredHelp)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(
            Arg::with_name("profile")
                .help("Profile to use when connecting to synapse.")
                .short("P")
                .long("profile")
                .default_value("default")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("server")
                .help("URI of the synapse client to connect to.")
                .short("s")
                .long("server")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("password")
                .help("Password to use when connecting to synapse.")
                .short("p")
                .long("password")
                .takes_value(true),
        )
        .subcommands(vec![
            SubCommand::with_name("add")
                .about("Adds torrents to synapse.")
                .arg(
                    Arg::with_name("directory")
                        .help("Custom directory to download the torrent to.")
                        .short("d")
                        .long("directory")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("pause")
                        .help("Whether or not the torrent should start paused.")
                        .short("P")
                        .long("pause"),
                )
                .arg(
                    Arg::with_name("import")
                        .help("Whether or not the torrent should be imported.")
                        .short("i")
                        .long("import"),
                )
                .arg(
                    Arg::with_name("files")
                        .help("Torrent files or magnets to add")
                        .multiple(true)
                        .short("f")
                        .long("files")
                        .required(true)
                        .index(1),
                ),
            SubCommand::with_name("del")
                .about("Deletes torrents from synapse.")
                .arg(
                    Arg::with_name("files")
                        .help("Delete files along with torrents.")
                        .short("f")
                        .long("files")
                        .default_value("false"),
                )
                .arg(
                    Arg::with_name("torrents")
                        .help("Names of torrents to delete.")
                        .multiple(true)
                        .short("t")
                        .long("torrents")
                        .required(true)
                        .index(1),
                ),
            SubCommand::with_name("dl")
                .about("Downloads a torrent.")
                .arg(
                    Arg::with_name("torrent")
                        .help("Name of torrent to download.")
                        .short("t")
                        .long("torrent")
                        .index(1)
                        .required(true),
                ),
            SubCommand::with_name("file")
                .about("Manipulate a file.")
                .arg(
                    Arg::with_name("file id")
                        .help("ID of file to use.")
                        .index(1)
                        .required(true),
                )
                .subcommands(vec![SubCommand::with_name("priority")
                    .about("Adjust a file's priority.")
                    .arg(
                        Arg::with_name("file pri")
                            .help("priority to set file to (0-5)")
                            .index(1)
                            .required(true),
                    )])
                .setting(AppSettings::SubcommandRequiredElseHelp),
            SubCommand::with_name("get")
                .about("Gets the specified resource.")
                .arg(
                    Arg::with_name("output")
                        .help("Output the results in the specified format.")
                        .short("o")
                        .long("output")
                        .possible_values(&["json", "text"])
                        .default_value("text"),
                )
                .arg(
                    Arg::with_name("id")
                        .help("ID of the resource.")
                        .index(1)
                        .required(true),
                ),
            SubCommand::with_name("list")
                .about("Lists resources of a given type in synapse.")
                .arg(
                    Arg::with_name("filter")
                        .help("Apply an array of json formatted criterion to the resources.")
                        .short("f")
                        .long("filter")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("kind")
                        .help("The kind of resource to list.")
                        .possible_values(&["torrent", "peer", "file", "server", "tracker", "piece"])
                        .default_value("torrent")
                        .short("k")
                        .long("kind"),
                )
                .arg(
                    Arg::with_name("output")
                        .help("Output the results in the specified format.")
                        .short("o")
                        .long("output")
                        .possible_values(&["json", "text"])
                        .default_value("text"),
                ),
            SubCommand::with_name("pause")
                .about("Pauses the given torrents.")
                .arg(
                    Arg::with_name("torrents")
                        .help("Names of torrents to pause.")
                        .required(true)
                        .multiple(true)
                        .short("t")
                        .long("torrents")
                        .index(1),
                ),
            SubCommand::with_name("resume")
                .about("Resumes the given torrents.")
                .arg(
                    Arg::with_name("torrents")
                        .help("Names of torrents to resume.")
                        .required(true)
                        .multiple(true)
                        .short("t")
                        .long("torrents")
                        .index(1),
                ),
            SubCommand::with_name("status").about("Server status"),
            SubCommand::with_name("watch")
                .about("Watches the specified resource, printing out updates.")
                .arg(
                    Arg::with_name("output")
                        .help("Output the results in the specified format.")
                        .short("o")
                        .long("output")
                        .possible_values(&["json", "text"])
                        .default_value("text"),
                )
                .arg(
                    Arg::with_name("completion")
                        .help("Polls until completion of torrent")
                        .short("c")
                        .long("completion"),
                )
                .arg(
                    Arg::with_name("id")
                        .help("ID of the resource.")
                        .index(1)
                        .required(true),
                ),
            SubCommand::with_name("torrent")
                .about("Manipulate torrent related resources")
                .arg(
                    Arg::with_name("torrent id")
                        .help("Name of torrent to download.")
                        .index(1),
                )
                .subcommands(vec![
                    SubCommand::with_name("tracker")
                        .about("Manipulate trackers for a torrent")
                        .subcommands(vec![
                            SubCommand::with_name("add")
                                .about("Add trackers to a torrent")
                                .arg(
                                    Arg::with_name("uris")
                                        .help("URIs of trackers to add")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                            SubCommand::with_name("remove")
                                .about("Remove trackers from a torrent")
                                .arg(
                                    Arg::with_name("tracker id")
                                        .help("ids of trackers to remove")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                            SubCommand::with_name("announce")
                                .about("Announce to a tracker of a torrent")
                                .arg(
                                    Arg::with_name("tracker id")
                                        .help("ids of trackers to announce to")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                        ])
                        .setting(AppSettings::SubcommandRequiredElseHelp),
                    SubCommand::with_name("peer")
                        .about("Manipulate peers for a torrent")
                        .subcommands(vec![
                            SubCommand::with_name("add")
                                .about("Add peers to a torrent")
                                .arg(
                                    Arg::with_name("peer ip")
                                        .help("IPs of peers to add")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                            SubCommand::with_name("remove")
                                .about("Remove peers from a torrent")
                                .arg(
                                    Arg::with_name("peer id")
                                        .help("ids of peers to remove")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                        ])
                        .setting(AppSettings::SubcommandRequiredElseHelp),
                    SubCommand::with_name("tag")
                        .about("Manipulate tags for a torrent")
                        .subcommands(vec![
                            SubCommand::with_name("add")
                                .about("Add tag to a torrent")
                                .arg(
                                    Arg::with_name("tag names")
                                        .help("Name of tags to add")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                            SubCommand::with_name("remove")
                                .about("Remove tags from a torrent")
                                .arg(
                                    Arg::with_name("tag names")
                                        .help("Name of tags to remove")
                                        .multiple(true)
                                        .index(1)
                                        .required(true),
                                ),
                        ])
                        .setting(AppSettings::SubcommandRequiredElseHelp),
                    SubCommand::with_name("priority")
                        .about("Change priority of a torrent")
                        .arg(
                            Arg::with_name("priority level")
                                .help("priority to set torrent to, 0-5")
                                .index(1)
                                .required(true),
                        ),
                    SubCommand::with_name("trackers").about("Prints a torrent's trackers"),
                    SubCommand::with_name("peers").about("Prints a torrent's peers"),
                    SubCommand::with_name("tags").about("Prints a torrent's tags"),
                    SubCommand::with_name("files").about("Prints a torrent's files"),
                ])
                .arg(
                    Arg::with_name("output")
                        .help("Output the results in the specified format.")
                        .short("o")
                        .long("output")
                        .possible_values(&["json", "text"])
                        .default_value("text"),
                )
                .setting(AppSettings::SubcommandRequiredElseHelp),
        ])
        .get_matches();

    let (mut server, mut pass) = match config.get(matches.value_of("profile").unwrap()) {
        Some(profile) => (profile.server.as_str(), profile.password.as_str()),
        None => {
            eprintln!(
                "Nonexistent profile {} referenced in argument!",
                matches.value_of("profile").unwrap()
            );
            process::exit(1);
        }
    };
    if let Some(url) = matches.value_of("server") {
        server = url;
    }
    if let Some(password) = matches.value_of("password") {
        pass = password;
    }
    let mut url = match Url::parse(server) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Server URL {} is not valid: {}", server, e);
            process::exit(1);
        }
    };
    url.query_pairs_mut().append_pair("password", pass);

    let client = match Client::new(url.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Failed to connect to synapse, ensure your URI and password are correct, {}",
                e.display_chain()
            );
            process::exit(1);
        }
    };

    if client.version().major != rpc::MAJOR_VERSION {
        eprintln!(
            "synapse RPC major version {} is not compatible with sycli RPC major version {}",
            client.version().major,
            rpc::MAJOR_VERSION
        );
        process::exit(1);
    }
    if client.version().minor < rpc::MINOR_VERSION {
        eprintln!(
            "synapse RPC minor version {} is not compatible with sycli RPC minor version {}",
            client.version().minor,
            rpc::MINOR_VERSION
        );
        process::exit(1);
    }

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
            let res = cmd::add(
                client,
                url.as_str(),
                files,
                args.value_of("directory"),
                !args.is_present("pause"),
                args.is_present("import"),
            );
            if let Err(e) = res {
                eprintln!("Failed to add torrents: {}", e.display_chain());
                process::exit(1);
            }
        }
        "del" => {
            let args = matches.subcommand_matches("del").unwrap();
            let artifacts = match args.value_of("files").unwrap() {
                "true" => true,
                _ => false,
            };
            let res = cmd::del(
                client,
                args.values_of("torrents").unwrap().collect(),
                artifacts,
            );
            if let Err(e) = res {
                eprintln!("Failed to delete torrents: {}", e.display_chain());
                process::exit(1);
            }
        }
        "dl" => {
            let args = matches.subcommand_matches("dl").unwrap();
            let res = cmd::dl(client, url.as_str(), args.value_of("torrent").unwrap());
            if let Err(e) = res {
                eprintln!("Failed to download torrent: {}", e.display_chain());
                process::exit(1);
            }
        }
        "file" => {
            let subcmd = matches.subcommand_matches("file").unwrap();
            let id = subcmd.value_of("file id").unwrap();
            match subcmd.subcommand_name().unwrap() {
                "priority" => {
                    let pscmd = subcmd.subcommand_matches("priority").unwrap();
                    let pri = pscmd.value_of("file pri").unwrap();
                    let res = cmd::set_file_pri(client, id, pri);
                    if let Err(e) = res {
                        eprintln!("Failed to download torrent: {}", e.display_chain());
                        process::exit(1);
                    }
                }
                _ => unreachable!(),
            }
        }
        "get" => {
            let args = matches.subcommand_matches("get").unwrap();
            let id = args.value_of("id").unwrap();
            let output = args.value_of("output").unwrap();
            let res = cmd::get(client, id, output);
            if let Err(e) = res {
                eprintln!("Failed to get resource: {}", e.display_chain());
                process::exit(1);
            }
        }
        "list" => {
            let args = matches.subcommand_matches("list").unwrap();
            let crit = args
                .value_of("filter")
                .and_then(|f| {
                    let single_crit = serde_json::from_str(f).map(|c| vec![c]).ok();
                    single_crit.or_else(|| serde_json::from_str(f).ok())
                })
                .unwrap_or_else(Vec::new);
            let kind = args.value_of("kind").unwrap();
            let output = args.value_of("output").unwrap();
            let res = cmd::list(client, kind, crit, output);
            if let Err(e) = res {
                eprintln!("Failed to list torrents: {}", e.display_chain());
                process::exit(1);
            }
        }
        "pause" => {
            let args = matches.subcommand_matches("pause").unwrap();
            let res = cmd::pause(client, args.values_of("torrents").unwrap().collect());
            if let Err(e) = res {
                eprintln!("Failed to pause torrents: {}", e.display_chain());
                process::exit(1);
            }
        }
        "resume" => {
            let args = matches.subcommand_matches("resume").unwrap();
            let res = cmd::resume(client, args.values_of("torrents").unwrap().collect());
            if let Err(e) = res {
                eprintln!("Failed to resume torrents: {}", e.display_chain());
                process::exit(1);
            }
        }
        "status" => {
            if let Err(e) = cmd::status(client) {
                eprintln!("Failed to get server status: {}", e.display_chain());
                process::exit(1);
            }
        }
        "torrent" => {
            let subcmd = matches.subcommand_matches("torrent").unwrap();
            let id = subcmd.value_of("torrent id").unwrap_or("none");
            let output = subcmd.value_of("output").unwrap();
            match subcmd.subcommand_name().unwrap() {
                "tracker" => {
                    let sscmd = subcmd.subcommand_matches("tracker").unwrap();
                    match sscmd.subcommand_name().unwrap() {
                        "add" => {
                            if let Err(e) = cmd::add_trackers(
                                client,
                                id,
                                sscmd
                                    .subcommand_matches("add")
                                    .unwrap()
                                    .values_of("uris")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to add trackers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        "remove" => {
                            if let Err(e) = cmd::remove_trackers(
                                client,
                                sscmd
                                    .subcommand_matches("remove")
                                    .unwrap()
                                    .values_of("tracker id")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to remove trackers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        "announce" => {
                            if let Err(e) = cmd::announce_trackers(
                                client,
                                sscmd
                                    .subcommand_matches("announce")
                                    .unwrap()
                                    .values_of("tracker id")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to remove trackers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                "peer" => {
                    let sscmd = subcmd.subcommand_matches("peer").unwrap();
                    match sscmd.subcommand_name().unwrap() {
                        "add" => {
                            if let Err(e) = cmd::add_peers(
                                client,
                                id,
                                sscmd
                                    .subcommand_matches("add")
                                    .unwrap()
                                    .values_of("peer ip")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to add peers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        "remove" => {
                            if let Err(e) = cmd::remove_peers(
                                client,
                                sscmd
                                    .subcommand_matches("remove")
                                    .unwrap()
                                    .values_of("peer id")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to remove peers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                "tag" => {
                    let sscmd = subcmd.subcommand_matches("tag").unwrap();
                    match sscmd.subcommand_name().unwrap() {
                        "add" => {
                            if let Err(e) = cmd::add_tags(
                                client,
                                id,
                                sscmd
                                    .subcommand_matches("add")
                                    .unwrap()
                                    .values_of("tag names")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to add peers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        "remove" => {
                            if let Err(e) = cmd::remove_tags(
                                client,
                                id,
                                sscmd
                                    .subcommand_matches("remove")
                                    .unwrap()
                                    .values_of("tag names")
                                    .unwrap()
                                    .collect(),
                            ) {
                                eprintln!("Failed to remove peers: {}", e.display_chain());
                                process::exit(1);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                "priority" => {
                    let pri = subcmd
                        .subcommand_matches("priority")
                        .unwrap()
                        .value_of("priority level")
                        .unwrap();
                    if let Err(e) = cmd::set_torrent_pri(client, id, pri) {
                        eprintln!("Failed to set torrent priority: {}", e.display_chain());
                        process::exit(1);
                    }
                }
                "files" => {
                    if let Err(e) = cmd::get_files(client, id, output) {
                        eprintln!("Failed to get torrent files: {}", e.display_chain());
                        process::exit(1);
                    }
                }
                "peers" => {
                    if let Err(e) = cmd::get_peers(client, id, output) {
                        eprintln!("Failed to get torrent peers: {}", e.display_chain());
                        process::exit(1);
                    }
                }
                "tags" => {
                    if let Err(e) = cmd::get_tags(client, id) {
                        eprintln!("Failed to get torrent tags: {}", e.display_chain());
                        process::exit(1);
                    }
                }
                "trackers" => {
                    if let Err(e) = cmd::get_trackers(client, id, output) {
                        eprintln!("Failed to get torrent trackers: {}", e.display_chain());
                        process::exit(1);
                    }
                }
                _ => unreachable!(),
            }
        }
        "watch" => {
            let args = matches.subcommand_matches("watch").unwrap();
            let id = args.value_of("id").unwrap();
            let output = args.value_of("output").unwrap();
            let completion = args.is_present("completion");
            let res = cmd::watch(client, id, output, completion);
            if let Err(e) = res {
                eprintln!("Failed to watch resource: {}", e.display_chain());
                process::exit(1);
            }
        }
        _ => {}
    }
}
