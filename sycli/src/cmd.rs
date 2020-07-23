use std::borrow::Cow;
use std::io::{self, Read};
use std::path::Path;
use std::{cmp, fs, mem};

use prettytable::format::consts::FORMAT_NO_LINESEP_WITH_TITLE as TABLE_FORMAT;
use prettytable::Table;
use reqwest::Client as HClient;
use sha1::{Digest, Sha1};
use url::Url;

use rpc::criterion::{Criterion, Operation, Value};
use rpc::message::{self, CMessage, SMessage};
use rpc::resource::{CResourceUpdate, Resource, ResourceKind, SResourceUpdate, Server};
use synapse_rpc as rpc;

use crate::client::Client;
use crate::error::{ErrorKind, Result, ResultExt};

pub fn add(
    mut c: Client,
    url: &str,
    files: Vec<&str>,
    dir: Option<&str>,
    start: bool,
    import: bool,
    output: &str,
) -> Result<()> {
    for file in files {
        if let Ok(magnet) = Url::parse(file) {
            add_magnet(&mut c, magnet, dir, start, output)?;
        } else {
            add_file(&mut c, url, file, dir, start, import, output)?;
        }
    }
    Ok(())
}

fn add_file(
    c: &mut Client,
    url: &str,
    file: &str,
    dir: Option<&str>,
    start: bool,
    import: bool,
    output: &str,
) -> Result<()> {
    let mut torrent = Vec::new();
    let mut f = fs::File::open(file).chain_err(|| ErrorKind::FileIO)?;
    f.read_to_end(&mut torrent)
        .chain_err(|| ErrorKind::FileIO)?;

    let msg = CMessage::UploadTorrent {
        serial: c.next_serial(),
        size: torrent.len() as u64,
        path: dir.as_ref().map(|d| format!("{}", d)),
        start,
        import,
    };
    let token = if let SMessage::TransferOffer { token, .. } = c.rr(msg)? {
        token
    } else {
        bail!("Failed to receieve transfer offer from synapse!");
    };
    let client = HClient::new();
    client
        .post(url)
        .bearer_auth(token)
        .body(torrent)
        .send()
        .chain_err(|| ErrorKind::HTTP)?;

    match c.recv()? {
        SMessage::ResourcesExtant { ids, .. } => {
            get_(c, ids[0].as_ref(), output)?;
        }
        SMessage::InvalidRequest(message::Error { reason, .. }) => {
            bail!("{}", reason);
        }
        SMessage::TransferFailed(message::Error { reason, .. }) => {
            bail!("{}", reason);
        }
        _ => {
            bail!("Failed to receieve upload acknowledgement from synapse");
        }
    }

    Ok(())
}
fn add_magnet(
    c: &mut Client,
    magnet: Url,
    dir: Option<&str>,
    start: bool,
    output: &str,
) -> Result<()> {
    let msg = CMessage::UploadMagnet {
        serial: c.next_serial(),
        uri: magnet.as_str().to_owned(),
        path: dir.as_ref().map(|d| format!("{}", d)),
        start,
    };
    match c.rr(msg)? {
        SMessage::ResourcesExtant { ids, .. } => {
            get_(c, ids[0].as_ref(), output)?;
        }
        SMessage::InvalidRequest(message::Error { reason, .. }) => {
            bail!("{}", reason);
        }
        _ => {
            bail!("Failed to receieve upload acknowledgement from synapse");
        }
    }
    Ok(())
}

pub fn del(mut c: Client, torrents: Vec<&str>, artifacts: bool) -> Result<()> {
    for torrent in torrents {
        del_torrent(&mut c, torrent, artifacts)?;
    }
    Ok(())
}

fn del_torrent(c: &mut Client, torrent: &str, artifacts: bool) -> Result<()> {
    let resources = search_torrent_name(c, torrent)?;
    if resources.len() == 1 {
        let msg = CMessage::RemoveResource {
            serial: c.next_serial(),
            id: resources[0].id().to_owned(),
            artifacts: Some(artifacts),
        };
        c.send(msg)?;
    } else if resources.is_empty() {
        eprintln!("Could not find any matching torrents for {}", torrent);
    } else {
        eprintln!(
            "Ambiguous results searching for {}. Potential alternatives include: ",
            torrent
        );
        for res in resources.into_iter().take(3) {
            if let Resource::Torrent(t) = res {
                eprintln!(
                    "{}",
                    t.name.unwrap_or_else(|| "[Unknown Magnet]".to_owned())
                );
            }
        }
    }
    Ok(())
}

pub fn dl(mut c: Client, url: &str, name: &str) -> Result<()> {
    let resources = search_torrent_name(&mut c, name)?;
    let token = get_server(&mut c)?.download_token;
    let files = if resources.len() == 1 {
        let msg = CMessage::FilterSubscribe {
            serial: c.next_serial(),
            kind: ResourceKind::File,
            criteria: vec![Criterion {
                field: "torrent_id".to_owned(),
                op: Operation::Eq,
                value: Value::S(resources[0].id().to_owned()),
            }],
        };
        if let SMessage::ResourcesExtant { ids, .. } = c.rr(msg)? {
            get_resources(&mut c, ids.iter().map(Cow::to_string).collect())?
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
                eprintln!(
                    "{}",
                    t.name.unwrap_or_else(|| "[Unknown Magnet]".to_owned())
                );
            }
        }
        return Ok(());
    };

    for file in files {
        let mut dl_url = Url::parse(url).unwrap();
        dl_url
            .path_segments_mut()
            .unwrap()
            .push("dl")
            .push(file.id());
        let digest = Sha1::digest(format!("{}{}", file.id(), token).as_bytes());
        let dl_token = base64::encode(&digest.as_slice());
        dl_url.set_query(Some(&format!("token={}", dl_token)));

        let client = HClient::new();
        let mut resp = client
            .get(dl_url.as_str())
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
    Ok(())
}

pub fn get(mut c: Client, id: &str, output: &str) -> Result<()> {
    get_(&mut c, id, output)
}

pub fn get_(c: &mut Client, id: &str, output: &str) -> Result<()> {
    let res = get_resources(c, vec![id.to_owned()])?;
    if res.is_empty() {
        bail!("Resource not found");
    }
    match output {
        "text" => {
            println!("{}", res[0]);
        }
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&res[0]).chain_err(|| ErrorKind::Serialization)?
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub fn list(mut c: Client, kind: &str, crit: Vec<Criterion>, output: &str) -> Result<()> {
    let k = match kind {
        "torrent" => ResourceKind::Torrent,
        "tracker" => ResourceKind::Tracker,
        "peer" => ResourceKind::Peer,
        "piece" => ResourceKind::Piece,
        "file" => ResourceKind::File,
        "server" => ResourceKind::Server,
        _ => bail!("Unexpected resource kind {}", kind),
    };
    let results = search(&mut c, k, crit)?;
    if output == "text" {
        let mut table = Table::new();
        table.set_format(*TABLE_FORMAT);
        match k {
            ResourceKind::Torrent => {
                table.set_titles(row!["Name", "Done", "DL", "UL", "DL RT", "UL RT", "Peers"]);
            }
            ResourceKind::Tracker => {
                table.set_titles(row!["URL", "Torrent", "Error"]);
            }
            ResourceKind::Peer => {
                table.set_titles(row!["IP", "Torrent", "DL RT", "UL RT"]);
            }
            ResourceKind::Piece => {
                table.set_titles(row!["Torrent", "DLd", "Avail"]);
            }
            ResourceKind::File => {
                table.set_titles(row!["Path", "Torrent", "Done", "Prio", "Avail"]);
            }
            ResourceKind::Server => {
                table.set_titles(row!["DL RT", "UL RT"]);
            }
        }

        #[cfg_attr(rustfmt, rustfmt_skip)]
        for res in results {
            match k {
                ResourceKind::Torrent => {
                    let t = res.as_torrent();
                    table.add_row(row![
                                  t.name.as_ref().map(|s| s.as_str()).unwrap_or("[Unknown Magnet]"),
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
                                  t.url.as_str(),
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
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&results).chain_err(|| ErrorKind::Serialization)?
        );
    }
    Ok(())
}

pub fn pause(mut c: Client, torrents: Vec<&str>) -> Result<()> {
    for torrent in torrents {
        pause_torrent(&mut c, torrent)?;
    }
    Ok(())
}

fn pause_torrent(c: &mut Client, torrent: &str) -> Result<()> {
    let resources = search_torrent_name(c, torrent)?;
    if resources.len() == 1 {
        let msg = CMessage::PauseTorrent {
            serial: c.next_serial(),
            id: resources[0].id().to_owned(),
        };
        c.send(msg)?;
    } else if resources.is_empty() {
        eprintln!("Could not find any matching torrents for {}", torrent);
    } else {
        eprintln!(
            "Ambiguous results searching for {}. Potential alternatives include: ",
            torrent
        );
        for res in resources.into_iter().take(3) {
            if let Resource::Torrent(t) = res {
                eprintln!(
                    "{}",
                    t.name.unwrap_or_else(|| "[Unknown Magnet]".to_owned())
                );
            }
        }
    }
    Ok(())
}

pub fn resume(mut c: Client, torrents: Vec<&str>) -> Result<()> {
    for torrent in torrents {
        resume_torrent(&mut c, torrent)?;
    }
    Ok(())
}

fn resume_torrent(c: &mut Client, torrent: &str) -> Result<()> {
    let resources = search_torrent_name(c, torrent)?;
    if resources.len() == 1 {
        let msg = CMessage::ResumeTorrent {
            serial: c.next_serial(),
            id: resources[0].id().to_owned(),
        };
        c.send(msg)?;
    } else if resources.is_empty() {
        eprintln!("Could not find any matching torrents for {}", torrent);
    } else {
        eprintln!(
            "Ambiguous results searching for {}. Potential alternatives include: ",
            torrent
        );
        for res in resources.into_iter().take(3) {
            if let Resource::Torrent(t) = res {
                eprintln!(
                    "{}",
                    t.name.unwrap_or_else(|| "[Unknown Magnet]".to_owned())
                );
            }
        }
    }
    Ok(())
}

pub fn watch(mut c: Client, id: &str, output: &str, completion: bool) -> Result<()> {
    let res = get_resources(&mut c, vec![id.to_owned()])?;
    if res.is_empty() {
        bail!("Resource not found");
    }

    let msg = CMessage::Subscribe {
        serial: c.next_serial(),
        ids: vec![id.to_owned()],
    };

    let resources = if let SMessage::UpdateResources { resources, .. } = c.rr(msg)? {
        resources
    } else {
        bail!("Failed to received torrent resource list!");
    };

    let mut results = Vec::new();
    for r in resources {
        if let SResourceUpdate::Resource(res) = r {
            results.push(res);
        } else {
            bail!("Failed to received full resource!");
        }
    }

    if results.is_empty() {
        bail!("Could not find specified resource!");
    }
    let mut res = results.remove(0).into_owned();
    if let Resource::Torrent(ref t) = res {
        if t.progress - 1.0 <= std::f32::EPSILON && completion {
            return Ok(());
        }
    }
    loop {
        match output {
            "text" => {
                println!("{}", res);
            }
            "json" => {
                println!(
                    "{}",
                    serde_json::to_string(&res).chain_err(|| ErrorKind::Serialization)?
                );
            }
            _ => unreachable!(),
        }
        loop {
            if let SMessage::UpdateResources { resources, .. } = c.recv()? {
                for r in resources {
                    if let SResourceUpdate::TorrentTransfer { progress, .. } = r {
                        if completion && progress == 1.0 {
                            return Ok(());
                        }
                    }
                    res.update(r);
                }
                break;
            }
        }
    }
}

pub fn move_torrent(mut c: Client, id: &str, dir: &str) -> Result<()> {
    let torrent = search_torrent_name(&mut c, id)?;
    if torrent.len() != 1 {
        bail!("Could not find appropriate torrent!");
    }
    let update = CMessage::UpdateResource {
        serial: c.next_serial(),
        resource: CResourceUpdate {
            id: torrent[0].id().to_owned(),
            path: Some(dir.to_owned()),
            ..Default::default()
        },
    };
    c.send(update)?;
    Ok(())
}

pub fn add_trackers(mut c: Client, id: &str, trackers: Vec<&str>) -> Result<()> {
    let torrent = search_torrent_name(&mut c, id)?;
    if torrent.len() != 1 {
        bail!("Could not find appropriate torrent!");
    }
    for tracker in trackers {
        if let Err(e) = add_tracker(&mut c, torrent[0].id(), tracker) {
            eprintln!("Failed to add tracker {}: {}", tracker, e);
        }
    }
    Ok(())
}

fn add_tracker(c: &mut Client, id: &str, tracker: &str) -> Result<()> {
    let msg = CMessage::AddTracker {
        serial: c.next_serial(),
        id: id.to_owned(),
        uri: tracker.to_owned(),
    };

    match c.rr(msg)? {
        SMessage::ResourcesExtant { .. } => Ok(()),
        SMessage::InvalidRequest(message::Error { reason, .. }) => {
            bail!("{}", reason);
        }
        _ => {
            bail!("Failed to receieve tracker extancy from synapse!");
        }
    }
}

pub fn remove_trackers(mut c: Client, trackers: Vec<&str>) -> Result<()> {
    for tracker in trackers {
        if let Err(e) = remove_res(&mut c, tracker) {
            eprintln!("Failed to remove tracker {}: {}", tracker, e);
        }
    }
    Ok(())
}

pub fn announce_trackers(mut c: Client, trackers: Vec<&str>) -> Result<()> {
    for id in trackers {
        let serial = c.next_serial();
        c.send(CMessage::UpdateTracker {
            serial,
            id: id.to_owned(),
        })?;
    }
    Ok(())
}

fn remove_res(c: &mut Client, res: &str) -> Result<()> {
    let msg = CMessage::RemoveResource {
        serial: c.next_serial(),
        id: res.to_owned(),
        artifacts: None,
    };
    match c.rr(msg)? {
        SMessage::ResourcesRemoved { .. } => Ok(()),
        SMessage::InvalidRequest(message::Error { reason, .. }) => {
            bail!("{}", reason);
        }
        _ => {
            bail!("Failed to receieve removal confirmation from synapse!");
        }
    }
}

pub fn add_peers(mut c: Client, id: &str, peers: Vec<&str>) -> Result<()> {
    let torrent = search_torrent_name(&mut c, id)?;
    if torrent.len() != 1 {
        bail!("Could not find appropriate torrent!");
    }
    for peer in peers {
        if let Err(e) = add_peer(&mut c, torrent[0].id(), peer) {
            eprintln!("Failed to add peer {}: {}", peer, e);
        }
    }
    Ok(())
}

fn add_peer(c: &mut Client, id: &str, peer: &str) -> Result<()> {
    let msg = CMessage::AddPeer {
        serial: c.next_serial(),
        id: id.to_owned(),
        ip: peer.to_owned(),
    };
    match c.rr(msg)? {
        SMessage::ResourcePending { .. } => Ok(()),
        SMessage::InvalidRequest(message::Error { reason, .. }) => {
            bail!("{}", reason);
        }
        _ => {
            bail!("Failed to peer extancy confirmation from synapse!");
        }
    }
}

pub fn remove_peers(mut c: Client, peers: Vec<&str>) -> Result<()> {
    for peer in peers {
        if let Err(e) = remove_res(&mut c, peer) {
            eprintln!("Failed to remove tracker {}: {}", peer, e);
        }
    }
    Ok(())
}

pub fn add_tags(mut c: Client, id: &str, tags: Vec<&str>) -> Result<()> {
    let mut resource = CResourceUpdate::default();
    let (id, mut tag_array) = get_tags_(&mut c, id)?;
    resource.id = id;
    for tag in tags {
        let t = tag.to_owned();
        if !tag_array.contains(&t) {
            tag_array.push(t);
        }
    }
    let tag_obj = serde_json::Value::Array(
        tag_array
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    );
    resource.user_data = Some(serde_json::json!({ "tags": tag_obj }));
    let msg = CMessage::UpdateResource {
        serial: c.next_serial(),
        resource,
    };
    c.send(msg)
}

pub fn remove_tags(mut c: Client, id: &str, tags: Vec<&str>) -> Result<()> {
    let mut resource = CResourceUpdate::default();
    let (id, mut tag_array) = get_tags_(&mut c, id)?;
    resource.id = id;
    tag_array.retain(|t| !tags.contains(&t.as_str()));
    let tag_obj = serde_json::Value::Array(
        tag_array
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    );
    resource.user_data = Some(serde_json::json!({ "tags": tag_obj }));
    let msg = CMessage::UpdateResource {
        serial: c.next_serial(),
        resource,
    };
    c.send(msg)
}

pub fn get_tags(mut c: Client, id: &str) -> Result<()> {
    let (_, tag_array) = get_tags_(&mut c, id)?;
    println!("Torrent tags: {:?}", tag_array);
    Ok(())
}

fn get_tags_(c: &mut Client, id: &str) -> Result<(String, Vec<String>)> {
    let mut sres = search_torrent_name(c, id)?;
    if sres.len() != 1 {
        bail!("Could not find appropriate torrent!");
    }
    let torrent = sres[0].as_torrent_mut();
    let prev_data = mem::replace(&mut torrent.user_data, serde_json::Value::Null);
    Ok((
        torrent.id.clone(),
        match prev_data.pointer("/tags") {
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|s| s.to_owned())
                .collect(),
            _ => vec![],
        },
    ))
}

pub fn set_torrent_pri(mut c: Client, id: &str, pri: &str) -> Result<()> {
    let p: u8 = pri.parse().chain_err(|| ErrorKind::Parse)?;
    let torrent = search_torrent_name(&mut c, id)?;
    if torrent.len() != 1 {
        bail!("Could not find appropriate torrent!");
    }
    let update = CMessage::UpdateResource {
        serial: c.next_serial(),
        resource: CResourceUpdate {
            id: torrent[0].id().to_owned(),
            priority: Some(p),
            ..Default::default()
        },
    };
    c.send(update)?;
    Ok(())
}

pub fn set_file_pri(mut c: Client, id: &str, pri: &str) -> Result<()> {
    let p: u8 = pri.parse().chain_err(|| ErrorKind::Parse)?;
    let update = CMessage::UpdateResource {
        serial: c.next_serial(),
        resource: CResourceUpdate {
            id: id.to_owned(),
            priority: Some(p),
            ..Default::default()
        },
    };
    c.send(update)?;
    Ok(())
}

pub fn get_files(mut c: Client, id: &str, output: &str) -> Result<()> {
    print_torrent_res(&mut c, id, ResourceKind::File, output)
}

pub fn get_peers(mut c: Client, id: &str, output: &str) -> Result<()> {
    print_torrent_res(&mut c, id, ResourceKind::Peer, output)
}

pub fn get_trackers(mut c: Client, id: &str, output: &str) -> Result<()> {
    print_torrent_res(&mut c, id, ResourceKind::Tracker, output)
}

fn print_torrent_res(c: &mut Client, id: &str, kind: ResourceKind, output: &str) -> Result<()> {
    let torrent = search_torrent_name(c, id)?;
    if torrent.len() != 1 {
        bail!("Could not find appropriate torrent!");
    }
    let files = search(
        c,
        kind,
        vec![Criterion {
            field: "torrent_id".to_owned(),
            op: Operation::Eq,
            value: Value::S(torrent[0].id().to_owned()),
        }],
    )?;
    for file in files {
        match output {
            "text" => {
                println!("{}", file);
            }
            "json" => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&file).chain_err(|| ErrorKind::Serialization)?
                );
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

pub fn status(mut c: Client) -> Result<()> {
    match search(&mut c, ResourceKind::Server, vec![])?.pop() {
        Some(Resource::Server(s)) => {
            let vi = s.id.find('-').unwrap();
            let version = &s.id[..vi];
            println!(
                "synapse v{}, RPC v{}.{}",
                version,
                c.version().major,
                c.version().minor
            );
            println!(
                "UL: {}/s, DL: {}/s, total UL: {}, total DL: {}",
                fmt_bytes(s.rate_up as f64),
                fmt_bytes(s.rate_down as f64),
                fmt_bytes(s.transferred_up as f64),
                fmt_bytes(s.transferred_down as f64),
            );
        }
        _ => {
            bail!("synapse server incorrectly reported server status!");
        }
    };
    Ok(())
}

fn get_server(c: &mut Client) -> Result<Server> {
    match search(c, ResourceKind::Server, vec![])?.pop() {
        Some(Resource::Server(s)) => Ok(s),
        _ => bail!("synapse server failed to server info!"),
    }
}

fn search_torrent_name(c: &mut Client, name: &str) -> Result<Vec<Resource>> {
    let mut res = search(
        c,
        ResourceKind::Torrent,
        vec![Criterion {
            field: "id".to_owned(),
            op: Operation::Eq,
            value: Value::S(name.to_owned()),
        }],
    )?;
    if res.is_empty() {
        res = search(
            c,
            ResourceKind::Torrent,
            vec![Criterion {
                field: "name".to_owned(),
                op: Operation::ILike,
                value: Value::S(format!("%{}%", name)),
            }],
        )?;
    }
    Ok(res)
}

fn search(c: &mut Client, kind: ResourceKind, criteria: Vec<Criterion>) -> Result<Vec<Resource>> {
    let s = c.next_serial();
    let msg = CMessage::FilterSubscribe {
        serial: s,
        kind,
        criteria,
    };
    if let SMessage::ResourcesExtant { ids, .. } = c.rr(msg)? {
        let ns = c.next_serial();
        c.send(CMessage::FilterUnsubscribe {
            serial: ns,
            filter_serial: s,
        })?;
        get_resources(c, ids.iter().map(Cow::to_string).collect())
    } else {
        bail!("Failed to receive extant resource list!");
    }
}

fn get_resources(c: &mut Client, ids: Vec<String>) -> Result<Vec<Resource>> {
    let msg = CMessage::Subscribe {
        serial: c.next_serial(),
        ids: ids.clone(),
    };
    let unsub = CMessage::Unsubscribe {
        serial: c.next_serial(),
        ids,
    };

    let resources = if let SMessage::UpdateResources { resources, .. } = c.rr(msg)? {
        resources
    } else {
        bail!("Failed to received torrent resource list!");
    };

    c.send(unsub)?;

    let mut results = Vec::new();
    for r in resources {
        if let SResourceUpdate::Resource(res) = r {
            results.push(res.into_owned());
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
        .unwrap()
        * 1_f64;
    let unit = units[exponent as usize];
    format!("{} {}", pretty_bytes, unit)
}
