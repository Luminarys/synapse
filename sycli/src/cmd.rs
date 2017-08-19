use std::fs;
use std::io::{Read};

use websocket::client::sync::Client as WClient;
use websocket::stream::Stream;
use websocket::message::OwnedMessage as WSMessage;
use reqwest::{Client as HClient, header};
use serde_json;

use rpc::message::{CMessage, SMessage};

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
    for file in files {
        add_file(&mut c, url, file, dir)?;
    }
    Ok(())
}

fn add_file<S: Stream>(c: &mut WClient<S>, url: &str, file: &str, dir: Option<&str>) -> Result<()> {
        let mut serial = Serial(0);
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

pub fn del<S: Stream>(c: WClient<S>) {
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
