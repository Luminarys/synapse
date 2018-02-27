extern crate rand;
extern crate serde_json as json;
extern crate synapse_rpc as rpc;
extern crate ws;

use std::collections::HashMap;
use std::time::Instant;
use rpc::resource::{Resource, ResourceKind, SResourceUpdate};
use rpc::message::{CMessage, Error, SMessage};

mod state;

struct Client {
    sender: ws::Sender,
    state: state::State,
}

impl Client {
    fn new(sender: ws::Sender) -> Client {
        Client {
            sender,
            state: state::State::new(),
        }
    }
}

impl ws::Handler for Client {
    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        match msg {
            ws::Message::Text(s) => {
                let time = Instant::now();
                let data: CMessage = json::from_str(&s).map_err(Box::new)?;
                println!("Processing msg");
                for resp in self.state.handle_client(0, data) {
                    let rs = json::to_string(&resp).unwrap();
                    self.sender.send(ws::Message::Text(rs))?;
                }
                let dur = time.elapsed();
                let nanos = dur.subsec_nanos() as u64;
                let ms = (1000 * 1000 * 1000 * dur.as_secs() + nanos) / (1000 * 1000);
                println!("Operation completed in: {} ms", ms);
                Ok(())
            }
            _ => Err(ws::Error::new(ws::ErrorKind::Internal, "non text frame!")),
        }
    }
}

fn main() {
    ws::listen("127.0.0.1:8412", Client::new).expect("Couldn't setup TCP listener");
}
