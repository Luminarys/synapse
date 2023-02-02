use sstream::SStream;
use url::Url;
use ws::protocol::Message as WSMessage;

use crate::rpc::message::{CMessage, SMessage, Version};

use crate::error::{ErrorKind, Result, ResultExt};

const OS_IN_PROGRESS_ERROR: i32 = 36;

pub struct Client {
    ws: ws::WebSocket<SStream>,
    version: Version,
    serial: u64,
}

impl Client {
    pub fn new(url: Url) -> Result<Client> {
        if !url.has_host() {
            bail!("Invalid websocket URL!");
        }
        for addr in url
            .socket_addrs(|| None)
            .chain_err(|| ErrorKind::Websocket)?
        {
            let mut stream = match url.scheme() {
                "ws" => {
                    if addr.is_ipv4() {
                        SStream::new_v4(None)
                    } else {
                        SStream::new_v6(None)
                    }
                }
                "wss" => {
                    if addr.is_ipv4() {
                        SStream::new_v4(Some(url.host_str().unwrap().to_owned()))
                    } else {
                        SStream::new_v6(Some(url.host_str().unwrap().to_owned()))
                    }
                }
                _ => bail!(""),
            }
            .chain_err(|| ErrorKind::Websocket)?;
            let connect_err = stream.connect(addr);
            match connect_err {
                Err(e) if e.raw_os_error() == Some(OS_IN_PROGRESS_ERROR) => {}
                other => other.chain_err(|| ErrorKind::Websocket)?,
            };
            stream
                .get_stream()
                .set_nonblocking(false)
                .chain_err(|| ErrorKind::Websocket)?;
            if let Ok((client, _response)) = ws::client(url.clone(), stream) {
                let mut c = Client {
                    ws: client,
                    serial: 0,
                    version: Version { major: 0, minor: 0 },
                };
                if let SMessage::RpcVersion(v) = c.recv()? {
                    c.version = v;
                    return Ok(c);
                } else {
                    bail!("Expected a version message on start!");
                }
            }
        }
        bail!("Could not connect to provided url!");
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
