use websocket::ClientBuilder;
use websocket::client::sync::Client as WSClient;
use websocket::stream::sync::NetworkStream;
use websocket::message::OwnedMessage as WSMessage;
use serde_json;

use rpc::message::{CMessage, SMessage, Version};

use error::{ErrorKind, Result, ResultExt};

pub struct Client {
    ws: WSClient<Box<NetworkStream + Send>>,
    version: Version,
    serial: u64,
}

impl Client {
    pub fn new(url: &str) -> Result<Client> {
        let client = ClientBuilder::new(url)
            .unwrap()
            .connect(None)
            .chain_err(|| ErrorKind::Websocket)?;
        let mut c = Client {
            ws: client,
            serial: 0,
            version: Version { major: 0, minor: 0 },
        };
        if let SMessage::RpcVersion(v) = c.recv()? {
            c.version = v;
            Ok(c)
        } else {
            bail!("Expected a version message on start!");
        }
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial - 1
    }

    pub fn send(&mut self, msg: CMessage) -> Result<()> {
        let msg_data = serde_json::to_string(&msg).chain_err(|| ErrorKind::Serialization)?;
        self.ws
            .send_message(&WSMessage::Text(msg_data))
            .chain_err(|| ErrorKind::Websocket)?;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<SMessage<'static>> {
        loop {
            match self.ws.recv_message() {
                Ok(WSMessage::Text(s)) => {
                    return serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization);
                }
                Ok(WSMessage::Ping(p)) => {
                    self.ws
                        .send_message(&WSMessage::Pong(p))
                        .chain_err(|| ErrorKind::Websocket)?;
                }
                Err(e) => return Err(e).chain_err(|| ErrorKind::Websocket),
                _ => {}
            };
        }
    }

    pub fn rr(&mut self, msg: CMessage) -> Result<SMessage<'static>> {
        self.send(msg)?;
        self.recv()
    }
}
