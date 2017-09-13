use std::collections::{HashMap, HashSet};
use std::mem;

use chrono::{DateTime, Utc, Duration};

use super::proto::message::{CMessage, SMessage, Error};
use super::proto::criterion::{self, Criterion};
use super::proto::resource::{Resource, ResourceKind, SResourceUpdate, merge_json};
use super::{CtlMessage, Message};
use torrent::info::Info;
use util::random_string;

// TODO: Figure out a way to reduce allocations
// in this entire file, ideally by taking pointers
// to existing heap allocated structures, and by
// inlining appropriately

pub struct Processor {
    subs: HashMap<String, HashSet<usize>>,
    filter_subs: HashMap<(usize, u64), Filter>,
    resources: HashMap<String, Resource>,
    // Index by resource kind
    kinds: Vec<HashSet<String>>,
    // Index by torrent ID
    torrent_idx: HashMap<String, HashSet<String>>,
    tokens: HashMap<String, BearerToken>,
}

struct Filter {
    kind: ResourceKind,
    criteria: Vec<Criterion>,
}

struct BearerToken {
    expiration: DateTime<Utc>,
    client: usize,
    serial: u64,
    kind: TransferKind,
}

#[derive(Clone)]
pub enum TransferKind {
    UploadTorrent {
        size: u64,
        path: Option<String>,
        start: bool,
    },
    UploadFiles { size: u64, path: String },
}

const EXPIRATION_DUR: i64 = 120;

impl Processor {
    pub fn new() -> Processor {
        Processor {
            subs: HashMap::new(),
            filter_subs: HashMap::new(),
            resources: HashMap::new(),
            tokens: HashMap::new(),
            torrent_idx: HashMap::new(),
            kinds: vec![HashSet::new(); 6],
        }
    }

    pub fn remove_expired_tokens(&mut self) {
        self.tokens.retain(|_, tok| tok.expiration > Utc::now())
    }

    pub fn get_dl(&self, id: &str) -> Option<String> {
        match self.resources.get(id) {
            Some(&Resource::File(ref f)) => {
                match self.resources.get(&f.torrent_id) {
                    Some(&Resource::Torrent(ref t)) => Some(t.path.clone() + "/" + &f.path),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn get_transfer(&mut self, tok: String) -> Option<(usize, u64, TransferKind)> {
        let mut res = None;
        let rem = match self.tokens.get(&tok) {
            Some(bt) => {
                match &bt.kind {
                    s @ &TransferKind::UploadTorrent { .. } => {
                        res = Some((bt.client, bt.serial, s.clone()));
                        false
                    }
                    _ => true,
                }
            }
            None => {
                res = None;
                false
            }
        };
        if rem {
            let tok = self.tokens.remove(&tok).unwrap();
            res = Some((tok.client, tok.serial, tok.kind));
        }
        res
    }

    pub fn handle_client(
        &mut self,
        client: usize,
        msg: CMessage,
    ) -> (Vec<SMessage>, Option<Message>) {
        let mut resp = Vec::new();
        let mut rmsg = None;
        match msg {
            CMessage::GetResources { serial, ids } => {
                let mut resources = Vec::new();
                for id in ids {
                    if let Some(r) = self.resources.get(&id) {
                        resources.push(SResourceUpdate::Resource(r));
                    } else {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown resource id {}", id),
                        }));
                    }
                }
                resp.push(SMessage::UpdateResources { resources });
            }
            CMessage::Subscribe { serial, ids } => {
                let mut resources = Vec::new();
                for id in ids {
                    if let Some(r) = self.resources.get(&id) {
                        resources.push(SResourceUpdate::Resource(r));
                        self.subs.get_mut(&id).map(|s| s.insert(client));
                    } else {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown resource id {}", id),
                        }));
                    }
                }
                resp.push(SMessage::UpdateResources { resources });
            }
            CMessage::Unsubscribe { ids, .. } => {
                for id in ids {
                    self.subs.get_mut(&id).map(|s| s.remove(&client));
                }
            }
            CMessage::UpdateResource {
                serial,
                mut resource,
            } => {
                let udo = mem::replace(&mut resource.user_data, None);
                if let Some(user_data) = udo {
                    if let Some(res) = self.resources.get_mut(&resource.id) {
                        merge_json(res.user_data(), &mut user_data.clone());
                        resp.push(SMessage::UpdateResources {
                            resources: vec![
                                SResourceUpdate::UserData {
                                    id: resource.id.clone(),
                                    kind: res.kind(),
                                    user_data: user_data,
                                },
                            ],
                        });
                    }
                }

                match self.resources.get(&resource.id) {
                    Some(&Resource::Torrent(_)) => {
                        rmsg = Some(Message::UpdateTorrent(resource));
                    }
                    Some(&Resource::File(ref f)) => {
                        // TODO: Validate other fields(make sure they're not present)
                        if let Some(p) = resource.priority {
                            rmsg = Some(Message::UpdateFile {
                                id: resource.id,
                                torrent_id: f.torrent_id.to_owned(),
                                priority: p,
                            });
                        }
                    }
                    Some(&Resource::Server(_)) => {
                        rmsg = Some(Message::UpdateServer {
                            id: resource.id,
                            throttle_up: resource.throttle_up,
                            throttle_down: resource.throttle_down,
                        });
                    }
                    Some(_) => {}
                    None => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown resource id {}", resource.id),
                        }));
                    }
                }
            }
            CMessage::RemoveResource { serial, id } => {
                match self.resources.get(&id) {
                    Some(&Resource::Torrent(_)) => {
                        rmsg = Some(Message::RemoveTorrent { id, client, serial });
                    }
                    Some(&Resource::Tracker(ref t)) => {
                        rmsg = Some(Message::RemoveTracker {
                            id,
                            torrent_id: t.id.to_owned(),
                            client,
                            serial,
                        });
                    }
                    Some(&Resource::Peer(ref p)) => {
                        rmsg = Some(Message::RemovePeer {
                            id,
                            torrent_id: p.id.to_owned(),
                            client,
                            serial,
                        });
                    }
                    Some(_) => {
                        resp.push(SMessage::InvalidResource(Error {
                            serial: Some(serial),
                            reason: format!("Only torrents, trackers, and peers may be removed"),
                        }));
                    }
                    None => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown resource id {}", id),
                        }));
                    }
                }
            }
            CMessage::FilterSubscribe {
                serial,
                kind,
                criteria,
            } => {
                let f = Filter { criteria, kind };
                {
                    let valid = &self.kinds[kind as usize];
                    let crit_res = f.criteria
                        .iter()
                        .find(|c| c.field == "torrent_id")
                        .and_then(|c| match &c.value {
                            &criterion::Value::S(ref s) => Some(s),
                            _ => None,
                        })
                        .and_then(|id| self.torrent_idx.get(id));

                    let mut ids = Vec::new();
                    if let Some(t) = crit_res {
                        for id in valid.intersection(t) {
                            let r = self.resources.get(id).unwrap();
                            if f.matches(r) {
                                ids.push(r.id());
                            }
                        }
                    } else {
                        for id in valid.iter() {
                            let r = self.resources.get(id).unwrap();
                            if f.matches(r) {
                                ids.push(r.id());
                            }
                        }
                    }
                    resp.push(SMessage::ResourcesExtant { serial, ids });
                }
                self.filter_subs.insert((client, serial), f);
            }
            CMessage::FilterUnsubscribe { filter_serial, .. } => {
                self.filter_subs.remove(&(client, filter_serial));
            }

            CMessage::PauseTorrent { serial, id } => {
                match self.resources.get(&id) {
                    Some(&Resource::Torrent(_)) => rmsg = Some(Message::Pause(id)),
                    Some(_) => {
                        resp.push(SMessage::InvalidResource(Error {
                            serial: Some(serial),
                            reason: "Only torrents can be paused".to_owned(),
                        }))
                    }
                    None => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("Unknown resource {}", id),
                        }))
                    }
                }
            }
            CMessage::ResumeTorrent { serial, id } => {
                match self.resources.get(&id) {
                    Some(&Resource::Torrent(_)) => rmsg = Some(Message::Resume(id)),
                    Some(_) => {
                        resp.push(SMessage::InvalidResource(Error {
                            serial: Some(serial),
                            reason: "Only torrents can be resumed".to_owned(),
                        }))
                    }
                    None => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("Unknown resource {}", id),
                        }))
                    }
                }
            }
            CMessage::UpdateTracker { serial, id } => {
                match self.resources.get(&id) {
                    Some(&Resource::Tracker(ref t)) => {
                        rmsg = Some(Message::UpdateTracker {
                            id,
                            torrent_id: t.torrent_id.clone(),
                        })
                    }
                    Some(_) => {
                        resp.push(SMessage::InvalidResource(Error {
                            serial: Some(serial),
                            reason: "UPDATE_TRACKER not used with tracker".to_owned(),
                        }))
                    }
                    None => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("Unknown resource {}", id),
                        }))
                    }
                }
            }
            CMessage::ValidateResources { serial, mut ids } => {
                ids.retain(|id| match self.resources.get(id) {
                    Some(&Resource::Torrent(_)) => true,
                    Some(_) => {
                        resp.push(SMessage::InvalidResource(Error {
                            serial: Some(serial),
                            reason: "Only torrents can be validated".to_owned(),
                        }));
                        false
                    }
                    None => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("Unknown resource {}", id),
                        }));
                        false
                    }
                });
                rmsg = Some(Message::Validate(ids));
            }
            CMessage::UploadTorrent {
                serial,
                size,
                path,
                start,
            } => {
                resp.push(self.new_transfer(
                    client,
                    serial,
                    TransferKind::UploadTorrent { size, path, start },
                ));
            }
            CMessage::UploadMagnet {
                serial,
                uri,
                path,
                start,
            } => {
                match Info::from_magnet(&uri) {
                    Ok(info) => {
                        rmsg = Some(Message::Torrent {
                            info,
                            path,
                            start,
                            client,
                            serial,
                        })
                    }
                    Err(e) => {
                        resp.push(SMessage::InvalidRequest(Error {
                            serial: Some(serial),
                            reason: format!("Invalid magnet: {}", e),
                        }));
                    }
                }
            }
            CMessage::UploadFiles { serial, size, path } => {
                resp.push(self.new_transfer(
                    client,
                    serial,
                    TransferKind::UploadFiles { size, path },
                ));
            }
        }
        (resp, rmsg)
    }

    pub fn handle_ctl(&mut self, msg: CtlMessage) -> Vec<(usize, SMessage)> {
        let mut msgs = Vec::new();
        match msg {
            CtlMessage::Extant(e) => {
                // TODO: Make this cleaner
                let mut ids = Vec::new();
                for r in e {
                    ids.push(r.id().to_owned());

                    self.subs.insert(r.id().to_owned(), HashSet::new());
                    let id = r.id().to_owned();

                    self.kinds[r.kind() as usize].insert(id.clone());

                    if let Some(tid) = r.torrent_id() {
                        if !self.torrent_idx.contains_key(tid) {
                            self.torrent_idx.insert(tid.to_owned(), HashSet::new());
                        }
                        self.torrent_idx.get_mut(tid).unwrap().insert(id.clone());
                    }

                    self.resources.insert(id, r);
                }
                // We have to make a new vec which points to the resource struct
                let mut rids = Vec::new();
                for id in ids {
                    rids.push(self.resources.get(&id).unwrap().id());
                }

                for ((client, serial), ids) in self.get_matching_filters(rids.into_iter()) {
                    msgs.push((client, SMessage::ResourcesExtant { serial, ids }));
                }
            }
            CtlMessage::Update(updates) => {
                let mut clients = HashMap::new();
                for update in updates {
                    for c in self.subs.get(update.id()).unwrap().iter() {
                        if !clients.contains_key(c) {
                            clients.insert(*c, Vec::new());
                        }
                        clients.get_mut(c).unwrap().push(update.clone());
                    }
                    self.resources
                        .get_mut(update.id())
                        .expect("Bad resource updated by a CtlMessage")
                        .update(update);
                }
                for (c, resources) in clients {
                    msgs.push((c, SMessage::UpdateResources { resources }));
                }
            }
            CtlMessage::Removed(r) => {
                for ((client, serial), ids) in
                    self.get_matching_filters(r.iter().map(|s| s.as_str()))
                {
                    msgs.push((
                        client,
                        SMessage::ResourcesRemoved {
                            serial,
                            ids: ids.into_iter().map(|s| s.to_owned()).collect(),
                        },
                    ));
                }

                for id in r {
                    let r = self.resources.remove(&id).unwrap();
                    self.kinds[r.kind() as usize].remove(&id);
                    // If this resource is part of a torrent, remove from index,
                    // if we haven't removed the entire torrent already.
                    // Otherwise, attempt to remove the resource itself from the
                    // torrent index, since it's either a torrent or a server(ignored).
                    if let Some(tid) = r.torrent_id() {
                        self.torrent_idx.get_mut(tid).map(|s| s.remove(&id));
                    } else {
                        self.torrent_idx.remove(&id);
                    }
                }
            }
            CtlMessage::ClientRemoved { id, client, serial } => {
                msgs.push((
                    client,
                    SMessage::ResourcesRemoved {
                        serial,
                        ids: vec![id],
                    },
                ));
            }
            CtlMessage::Uploaded { id, serial, client } => {
                if let Some(r) = self.resources.get(&id) {
                    msgs.push((
                        client,
                        SMessage::ResourcesExtant {
                            serial,
                            ids: vec![r.id()],
                        },
                    ))
                } else {
                    debug!("Failed to get resource uploaded: {}!", id);
                }
            }
            CtlMessage::Shutdown => unreachable!(),
        }
        msgs
    }

    pub fn remove_client(&mut self, client: usize) {
        for (_, sub) in self.subs.iter_mut() {
            sub.remove(&client);
        }
        self.filter_subs.retain(|&(c, _), _| c != client);
    }

    /// Produces a map of the form Map<(Client ID, Serial), messages)>.
    fn get_matching_filters<'a, I: Iterator<Item = &'a str>>(
        &'a self,
        ids: I,
    ) -> HashMap<(usize, u64), Vec<&'a str>> {
        let mut matched = HashMap::new();
        for id in ids {
            let res = self.resources.get(id).expect(
                "Bad resource requested from a CtlMessage",
            );
            for (k, f) in self.filter_subs.iter() {
                if f.kind == res.kind() && f.matches(&res) {
                    if !matched.contains_key(k) {
                        matched.insert(k.clone(), Vec::new());
                    }
                    matched.get_mut(&k).unwrap().push(id);
                }
            }
        }
        matched
    }

    fn new_transfer(&mut self, client: usize, serial: u64, kind: TransferKind) -> SMessage {
        let expiration = Utc::now() + Duration::seconds(EXPIRATION_DUR);
        let tok = random_string(15);
        self.tokens.insert(
            tok.clone(),
            BearerToken {
                expiration,
                kind,
                serial,
                client,
            },
        );
        SMessage::TransferOffer {
            serial,
            expires: expiration,
            token: tok,
            // TODO: Get this
            size: 0,
        }
    }
}

impl Filter {
    pub fn matches(&self, r: &Resource) -> bool {
        self.criteria.iter().all(|c| c.matches(r))
    }
}
