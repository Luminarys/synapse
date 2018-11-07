use ws;
use ws::client::AutoStream;
use ws::protocol::Message as WSMessage;
use serde_json;
use url::Url;

use rpc;
use rpc::message::{SMessage, CMessage, Version};

use error::{ErrorKind, Result, ResultExt};

pub struct Client {
    ws: ws::WebSocket<AutoStream>,
    version: Version,
    serial: u64,
}

impl Client {
    pub fn new(url: Url) -> Result<Client> {
        let client = ws::connect(url).chain_err(|| ErrorKind::Websocket)?.0;
        let mut c = Client {
            ws: client,
            serial: 0,
            version: Version {
                major: rpc::MAJOR_VERSION,
                minor: rpc::MINOR_VERSION
            },
        };

        match c.recv()? {
            SMessage::RpcVersion(v) => {
                if c.version.major != v.major {
                    bail!("RPC major version missmatch (server: {}, client: {})",
                        v.major,
                        c.version.major
                    )
                } else if c.version.minor < v.minor {
                    bail!("RPC minor version missmatch (server: {}, client: {})",
                        v.minor,
                        c.version.minor
                    )
                } else {
                    Ok(c)
                }
            },
            _ => bail!("expected version message at start")
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
            .write_message(WSMessage::Text(msg_data))
            .chain_err(|| ErrorKind::Websocket)?;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<SMessage<'static>> {
        loop {
            match self.ws.read_message() {
                Ok(WSMessage::Text(s)) => {
                    return serde_json::from_str(&s).chain_err(|| ErrorKind::Deserialization);
                }
                Ok(WSMessage::Ping(p)) => {
                    self.ws
                        .write_message(WSMessage::Pong(p))
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
