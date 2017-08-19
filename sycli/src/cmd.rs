use std::fs;
use std::io::{Read};

use websocket::client::sync::Client as WClient;
use websocket::stream::Stream;
use websocket::message::OwnedMessage as WSMessage;
use reqwest::{Client as HClient, header};
use serde_json;

use rpc::message::{CMessage, SMessage};
use rpc::criterion::{Criterion, Value, Operation};
use rpc::resource::{Resource, ResourceKind, SResourceUpdate};

error_chain! {
    errors {
        FileRead {
            description("Failed to read file")
                display("Failed to read file")
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

pub fn add<S: Stream>(mut c: WClient<S>, url: &str, files: Vec<&str>, dir: Option<&str>) -> Result<()> {
    let mut serial = Serial(0);
    for file in files {
        add_file(&mut c, &mut serial, url, file, dir)?;
    }
    Ok(())
}

fn add_file<S: Stream>(c: &mut WClient<S>, serial: &mut Serial, url: &str, file: &str, dir: Option<&str>) -> Result<()> {
        let mut torrent = Vec::new();
        let mut f = fs::File::open(file).chain_err(|| ErrorKind::FileRead)?;
        f.read_to_end(&mut torrent).chain_err(|| ErrorKind::FileRead)?;

        let msg = CMessage::UploadTorrent {
            serial: serial.next(),
            size: torrent.len() as u64,
            path: dir.as_ref().map(|d| format!("{}", d)),
        };
        let msg_data = serde_json::to_string(&msg).chain_err(|| ErrorKind::Serialization)?;
        let wsmsg = WSMessage::Text(msg_data);
        c.send_message(&wsmsg).chain_err(|| ErrorKind::Websocket)?;
        let mut smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
            WSMessage::Text(s) => serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization)?,
            // TODO: Handle Ping here
            _ => unimplemented!(),
        };
        let token = if let SMessage::TransferOffer { token, .. } = smsg {
            token
        } else {
            bail!("Failed to receieve transfer offer from synapse!");
        };
        let client = HClient::new().chain_err(|| ErrorKind::HTTP)?;
        client.post(url).chain_err(|| ErrorKind::HTTP)?
            .header(header::Authorization(header::Bearer{token}))
            .body(torrent)
            .send().chain_err(|| ErrorKind::HTTP)?;

        smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
            WSMessage::Text(s) => serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization)?,
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
    let resources = search(c, serial, torrent)?;
    if resources.len() == 1 {
        let msg = CMessage::RemoveResource {
            serial: serial.next(),
            id: resources[0].id().to_owned(),
        };
        let msg_data = serde_json::to_string(&msg).chain_err(|| ErrorKind::Serialization)?;
        c.send_message(&WSMessage::Text(msg_data)).chain_err(|| ErrorKind::Websocket)?;
    } else if resources.is_empty() {
        eprintln!("Could not find any matching torrents for {}", torrent);
    } else {
        eprintln!("Ambiguous results searching for {}. Potential alternatives include: ", torrent);
        for res in resources.into_iter().take(3) {
            if let Resource::Torrent(t) = res {
                eprintln!("{}", t.name);
            }
        }
    }
    Ok(())
}

fn search<S: Stream>(c: &mut WClient<S>, serial: &mut Serial, name: &str) -> Result<Vec<Resource>> {
    let s = serial.next();
    let msg = CMessage::FilterSubscribe {
        serial: s,
        kind: ResourceKind::Torrent,
        criteria: vec![Criterion {
            field: "name".to_owned(),
            op: Operation::ILike,
            value: Value::S(format!("%{}%", name)),
        }],
    };
    let msg_data = serde_json::to_string(&msg).chain_err(|| ErrorKind::Serialization)?;
    c.send_message(&WSMessage::Text(msg_data)).chain_err(|| ErrorKind::Websocket)?;
    let mut smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
        WSMessage::Text(s) => serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization)?,
        _ => unimplemented!(),
    };
    if let SMessage::OResourcesExtant { ids, .. } = smsg {
        let msg_data = serde_json::to_string(&CMessage::Subscribe {
            serial: serial.next(),
            ids,
        }).chain_err(|| ErrorKind::Serialization)?;
        c.send_message(&WSMessage::Text(msg_data)).chain_err(|| ErrorKind::Websocket)?;
    } else {
        bail!("Failed to receive extant resource list!");
    }

    smsg = match c.recv_message().chain_err(|| ErrorKind::Websocket)? {
        WSMessage::Text(s) => serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization)?,
        _ => unimplemented!(),
    };
    let resources = if let SMessage::UpdateResources{ resources } = smsg {
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

pub fn dl<S: Stream>(c: WClient<S>) {
}

pub fn list<S: Stream>(c: WClient<S>) {
}

pub fn rate<S: Stream>(c: WClient<S>) {
}

pub fn start<S: Stream>(c: WClient<S>) {
}

pub fn stop<S: Stream>(c: WClient<S>) {
}
