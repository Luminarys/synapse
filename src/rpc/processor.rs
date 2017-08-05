use std::collections::{HashMap, HashSet};

use super::proto::message::{CMessage, SMessage};
use super::proto::criterion::{Criterion, Operation, Value, ResourceKind};
use super::proto::resource::{Resource, Torrent};

pub struct Processor {
    subs: HashMap<u64, HashSet<u64>>,
    filter_subs: HashMap<u64, Filter>,
    resources: HashMap<u64, Resource>,
}

struct Filter {
    serial: u64,
    criteria: Vec<Criterion>,
    client: u64,
}

impl Processor {
    pub fn new() -> Processor {
        Processor {
            subs: HashMap::new(),
            filter_subs: HashMap::new(),
            resources: HashMap::new(),
        }
    }

    pub fn handle_client(&mut self, msg: CMessage) -> Vec<SMessage> {
        let mut resp = Vec::new();
        resp
    }
}
