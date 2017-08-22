use std::process;

use websocket::ClientBuilder;
use websocket::client::sync::Client as WSClient;
use websocket::stream::sync::NetworkStream;
use websocket::message::OwnedMessage as WSMessage;
use serde_json;

use rpc::message::{CMessage, SMessage};

use error::{Result, ResultExt, ErrorKind};

pub struct Client {
    ws: WSClient<Box<NetworkStream + Send>>,
    serial: u64,
}

impl Client {
    pub fn new(url: &str) -> Client {
        let client = match ClientBuilder::new(url).unwrap().connect(None) {
            Ok(c) => c,
            Err(_) => {
                eprintln!("Couldn't connect to synapse!");
                process::exit(1);
            }
        };
        Client {
            ws: client,
            serial: 0,
        }
    }

    pub fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial - 1
    }

    pub fn send(&mut self, msg: CMessage) -> Result<()> {
        let msg_data = serde_json::to_string(&msg).chain_err(
            || ErrorKind::Serialization,
        )?;
        self.ws.send_message(&WSMessage::Text(msg_data)).chain_err(
            || {
                ErrorKind::Websocket
            },
        )?;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<SMessage> {
        loop {
            match self.ws.recv_message().chain_err(|| ErrorKind::Websocket)? {
                WSMessage::Text(s) => {
                    return serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization);
                }
                _ => {}
            };
        }
    }

    pub fn rr(&mut self, msg: CMessage) -> Result<SMessage> {
        self.send(msg)?;
        self.recv()
    }
}
