use std::{fs, cmp};
use std::path::Path;
use std::io::{self, Read};

use websocket::client::sync::Client as WClient;
use websocket::stream::Stream;
use websocket::message::OwnedMessage as WSMessage;
use reqwest::{Client as HClient, header};
use serde_json;
use prettytable::Table;

use rpc::message::{CMessage, SMessage};
use rpc::criterion::{Criterion, Value, Operation};
use rpc::resource::{Resource, ResourceKind, SResourceUpdate};

error_chain! {
    errors {
        FileIO {
            description("Failed to perform file IO")
                display("Failed to perform file IO")
        }
        Serialization {
            description("Failed to serialize structure")
                display("Failed to serialize structure")
        }
        Deserialization {
            description("Failed to deserialize structure")
                display("Failed to deserialize structure")
        }
        Websocket {
            description("Failed to handle websocket client")
                display("Failed to handle websocket client")
        }
        HTTP {
            description("HTTP transfer failed")
                display("HTTP transfer failed")
        }
    }
}

struct Serial(u64);
impl Serial {
    fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0 - 1
    }
}

pub fn add<S: Stream>(
    mut c: WClient<S>,
    url: &str,
    files: Vec<&str>,
    dir: Option<&str>,
) -> Result<()> {
    let mut serial = Serial(0);
    for file in files {
        add_file(&mut c, &mut serial, url, file, dir)?;
    }
    Ok(())
}

fn add_file<S: Stream>(
    c: &mut WClient<S>,
    serial: &mut Serial,
    url: &str,
    file: &str,
    dir: Option<&str>,
) -> Result<()> {
    let mut torrent = Vec::new();
    let mut f = fs::File::open(file).chain_err(|| ErrorKind::FileIO)?;
    f.read_to_end(&mut torrent).chain_err(|| ErrorKind::FileIO)?;

    let msg = CMessage::UploadTorrent {
        serial: serial.next(),
        size: torrent.len() as u64,
        path: dir.as_ref().map(|d| format!("{}", d)),
    };
    let msg_data = serde_json::to_string(&msg).chain_err(
        || ErrorKind::Serialization,
    )?;
    let wsmsg = WSMessage::Text(msg_data);
    c.send_message(&wsmsg).chain_err(|| ErrorKind::Websocket)?;
    let mut smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
        WSMessage::Text(s) => {
            serde_json::from_str(&s).chain_err(
                || ErrorKind::Deserialization,
            )?
        }
        // TODO: Handle Ping here
        _ => unimplemented!(),
    };
    let token = if let SMessage::TransferOffer { token, .. } = smsg {
        token
    } else {
        bail!("Failed to receieve transfer offer from synapse!");
    };
    let client = HClient::new().chain_err(|| ErrorKind::HTTP)?;
    client
        .post(url)
        .chain_err(|| ErrorKind::HTTP)?
        .header(header::Authorization(header::Bearer { token }))
        .body(torrent)
        .send()
        .chain_err(|| ErrorKind::HTTP)?;

    smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
        WSMessage::Text(s) => {
            serde_json::from_str(&s).chain_err(
                || ErrorKind::Deserialization,
            )?
        }
        _ => unimplemented!(),
    };
    if let SMessage::OResourcesExtant { .. } = smsg {
    } else {
        bail!("Failed to receieve upload acknowledgement from synapse!");
    };

    Ok(())
}

pub fn del<S: Stream>(mut c: WClient<S>, torrents: Vec<&str>) -> Result<()> {
    let mut serial = Serial(0);
    for torrent in torrents {
        del_torrent(&mut c, &mut serial, torrent)?;
    }
    Ok(())
}

fn del_torrent<S: Stream>(c: &mut WClient<S>, serial: &mut Serial, torrent: &str) -> Result<()> {
    let resources = search_torrent_name(c, serial, torrent)?;
    if resources.len() == 1 {
        let msg = CMessage::RemoveResource {
            serial: serial.next(),
            id: resources[0].id().to_owned(),
        };
        let msg_data = serde_json::to_string(&msg).chain_err(
            || ErrorKind::Serialization,
        )?;
        c.send_message(&WSMessage::Text(msg_data)).chain_err(|| {
            ErrorKind::Websocket
        })?;
    } else if resources.is_empty() {
        eprintln!("Could not find any matching torrents for {}", torrent);
    } else {
        eprintln!(
            "Ambiguous results searching for {}. Potential alternatives include: ",
            torrent
        );
        for res in resources.into_iter().take(3) {
            if let Resource::Torrent(t) = res {
                eprintln!("{}", t.name);
            }
        }
    }
    Ok(())
}

pub fn dl<S: Stream>(mut c: WClient<S>, url: &str, name: &str) -> Result<()> {
    let mut serial = Serial(0);
    let resources = search_torrent_name(&mut c, &mut serial, name)?;
    let files = if resources.len() == 1 {
        let msg = CMessage::FilterSubscribe {
            serial: serial.next(),
            kind: ResourceKind::File,
            criteria: vec![
                Criterion {
                    field: "torrent_id".to_owned(),
                    op: Operation::Eq,
                    value: Value::S(resources[0].id().to_owned()),
                },
            ],
        };
        let msg_data = serde_json::to_string(&msg).chain_err(
            || ErrorKind::Serialization,
        )?;
        c.send_message(&WSMessage::Text(msg_data)).chain_err(|| {
            ErrorKind::Websocket
        })?;

        let smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
            WSMessage::Text(s) => {
                serde_json::from_str(&s).chain_err(
                    || ErrorKind::Deserialization,
                )?
            }
            _ => unimplemented!(),
        };
        if let SMessage::OResourcesExtant { ids, .. } = smsg {
            get_resources(&mut c, &mut serial, ids)?
        } else {
            bail!("Could not get files for torrent!");
        }
    } else if resources.is_empty() {
        eprintln!("Could not find any matching torrents for {}", name);
        return Ok(());
    } else {
        eprintln!(
            "Ambiguous results searching for {}. Potential alternatives include: ",
            name
        );
        for res in resources.into_iter().take(3) {
            if let Resource::Torrent(t) = res {
                eprintln!("{}", t.name);
            }
        }
        return Ok(());
    };

    for file in files {
        let msg = CMessage::DownloadFile {
            serial: serial.next(),
            id: file.id().to_owned(),
        };
        let msg_data = serde_json::to_string(&msg).chain_err(
            || ErrorKind::Serialization,
        )?;
        c.send_message(&WSMessage::Text(msg_data)).chain_err(|| {
            ErrorKind::Websocket
        })?;

        let smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
            WSMessage::Text(s) => {
                serde_json::from_str(&s).chain_err(
                    || ErrorKind::Deserialization,
                )?
            }
            _ => unimplemented!(),
        };
        if let SMessage::TransferOffer { token, .. } = smsg {
            let client = HClient::new().chain_err(|| ErrorKind::HTTP)?;
            let mut resp = client
                .get(url)
                .chain_err(|| ErrorKind::HTTP)?
                .header(header::Authorization(header::Bearer { token }))
                .send()
                .chain_err(|| ErrorKind::HTTP)?;
            if let Resource::File(f) = file {
                let p = Path::new(&f.path);
                if let Some(par) = p.parent() {
                    fs::create_dir_all(par).chain_err(|| ErrorKind::FileIO)?;
                }
                let mut f = fs::File::create(p).chain_err(|| ErrorKind::FileIO)?;
                io::copy(&mut resp, &mut f).chain_err(|| ErrorKind::FileIO)?;
            } else {
                bail!("Expected a file resource");
            }
        }
    }
    Ok(())
}

pub fn list<S: Stream>(
    mut c: WClient<S>,
    kind: &str,
    crit: Vec<Criterion>,
    output: &str,
) -> Result<()> {
    let k = match kind {
        "torrent" => ResourceKind::Torrent,
        "tracker" => ResourceKind::Tracker,
        "peer" => ResourceKind::Peer,
        "piece" => ResourceKind::Piece,
        "file" => ResourceKind::File,
        "server" => ResourceKind::Server,
        _ => bail!("Unexpected resource kind {}", kind),
    };
    let mut serial = Serial(0);
    let results = search(&mut c, &mut serial, k, crit)?;
    let mut table = Table::new();
    if output == "text" {
        match k {
            ResourceKind::Torrent => {
                table.add_row(row!["Name", "Done", "DL", "UL", "DL RT", "UL RT", "Peers"]);
            }
            ResourceKind::Tracker => {
                table.add_row(row!["URL", "Torrent", "Error"]);
            }
            ResourceKind::Peer => {
                table.add_row(row!["IP", "Torrent", "DL RT", "UL RT"]);
            }
            ResourceKind::Piece => {
                table.add_row(row!["Torrent", "DLd", "Avail"]);
            }
            ResourceKind::File => {
                table.add_row(row!["Path", "Torrent", "Done", "Prio", "Avail"]);
            }
            ResourceKind::Server => {
                table.add_row(row!["DL RT", "UL RT"]);
            }
        }
    }
    for res in results {
        match k {
            ResourceKind::Torrent => {
                let t = res.as_torrent();
                table.add_row(row![
                    t.name,
                    format!("{:.2}%", t.progress * 100.),
                    fmt_bytes(t.transferred_down as f64),
                    fmt_bytes(t.transferred_up as f64),
                    fmt_bytes(t.rate_down as f64) + "/s",
                    fmt_bytes(t.rate_up as f64) + "/s",
                    t.peers
                ]);
            }
            ResourceKind::Tracker => {
                let t = res.as_tracker();
                table.add_row(row![
                    t.url,
                    t.torrent_id,
                    t.error.as_ref().map(|s| s.as_str()).unwrap_or("")
                ]);
            }
            ResourceKind::Peer => {
                let p = res.as_peer();
                let rd = fmt_bytes(p.rate_down as f64) + "/s";
                let ru = fmt_bytes(p.rate_up as f64) + "/s";
                table.add_row(row![p.ip, p.torrent_id, rd, ru]);
            }
            ResourceKind::Piece => {
                let p = res.as_piece();
                table.add_row(row![p.torrent_id, p.downloaded, p.available]);
            }
            ResourceKind::File => {
                let f = res.as_file();
                table.add_row(row![
                    f.path,
                    f.torrent_id,
                    format!("{:.2}%", f.progress as f64 * 100.),
                    f.priority,
                    format!("{:.2}%", f.availability as f64 * 100.)
                ]);
            }
            ResourceKind::Server => {
                let s = res.as_server();
                let rd = fmt_bytes(s.rate_down as f64) + "/s";
                let ru = fmt_bytes(s.rate_up as f64) + "/s";
                table.add_row(row![rd, ru]);
            }
        }
    }
    table.printstd();
    Ok(())
}

pub fn rate<S: Stream>(c: WClient<S>) {}

pub fn start<S: Stream>(c: WClient<S>) {}

pub fn stop<S: Stream>(c: WClient<S>) {}

fn search_torrent_name<S: Stream>(
    c: &mut WClient<S>,
    serial: &mut Serial,
    name: &str,
) -> Result<Vec<Resource>> {
    search(
        c,
        serial,
        ResourceKind::Torrent,
        vec![
            Criterion {
                field: "name".to_owned(),
                op: Operation::ILike,
                value: Value::S(format!("%{}%", name)),
            },
        ],
    )
}

fn search<S: Stream>(
    c: &mut WClient<S>,
    serial: &mut Serial,
    kind: ResourceKind,
    criteria: Vec<Criterion>,
) -> Result<Vec<Resource>> {
    let s = serial.next();
    let msg = CMessage::FilterSubscribe {
        serial: s,
        kind,
        criteria,
    };
    let msg_data = serde_json::to_string(&msg).chain_err(
        || ErrorKind::Serialization,
    )?;
    c.send_message(&WSMessage::Text(msg_data)).chain_err(|| {
        ErrorKind::Websocket
    })?;
    let smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
        WSMessage::Text(s) => {
            serde_json::from_str(&s).chain_err(
                || ErrorKind::Deserialization,
            )?
        }
        _ => unimplemented!(),
    };
    if let SMessage::OResourcesExtant { ids, .. } = smsg {
        get_resources(c, serial, ids)
    } else {
        bail!("Failed to receive extant resource list!");
    }
}

fn get_resources<S: Stream>(
    c: &mut WClient<S>,
    serial: &mut Serial,
    ids: Vec<String>,
) -> Result<Vec<Resource>> {
    let msg_data = serde_json::to_string(&CMessage::Subscribe {
        serial: serial.next(),
        ids,
    }).chain_err(|| ErrorKind::Serialization)?;
    c.send_message(&WSMessage::Text(msg_data)).chain_err(|| {
        ErrorKind::Websocket
    })?;
    let smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
        WSMessage::Text(s) => {
            serde_json::from_str(&s).chain_err(
                || ErrorKind::Deserialization,
            )?
        }
        _ => unimplemented!(),
    };
    let resources = if let SMessage::UpdateResources { resources } = smsg {
        resources
    } else {
        bail!("Failed to received torrent resource list!");
    };

    let mut results = Vec::new();
    for r in resources {
        if let SResourceUpdate::OResource(res) = r {
            results.push(res);
        } else {
            bail!("Failed to received full resource!");
        }
    }
    Ok(results)
}

fn fmt_bytes(num: f64) -> String {
    let num = num.abs();
    let units = ["B", "kiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"];
    if num < 1_f64 {
        return format!("{} {}", num, "B");
    }
    let delimiter = 1024_f64;
    let exponent = cmp::min(
        (num.ln() / delimiter.ln()).floor() as i32,
        (units.len() - 1) as i32,
    );
    let pretty_bytes = format!("{:.2}", num / delimiter.powi(exponent))
        .parse::<f64>()
        .unwrap() * 1_f64;
    let unit = units[exponent as usize];
    format!("{} {}", pretty_bytes, unit)
}
