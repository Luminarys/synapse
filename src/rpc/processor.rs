use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc, Duration};

use super::proto::message::{CMessage, SMessage, Error};
use super::proto::criterion::{Criterion, Operation, Value, ResourceKind, Filter as FTrait};
use super::proto::resource::{Resource, Torrent, CResourceUpdate, SResourceUpdate};
use super::Message;
use util::random_string;

pub struct Processor {
    subs: HashMap<u64, HashSet<usize>>,
    filter_subs: HashMap<u64, Filter>,
    resources: HashMap<u64, Resource>,
    tokens: HashMap<String, BearerToken>,
}

struct Filter {
    criteria: Vec<Criterion>,
    client: usize,
}

struct BearerToken {
    expiration: DateTime<Utc>,
    kind: TransferKind,
}

enum TransferKind {
    UploadTorrent { size: u64, path: Option<String> },
    UploadFiles { size: u64, path: String },
    DownloadFile { path: String },
}

impl Processor {
    pub fn new() -> Processor {
        Processor {
            subs: HashMap::new(),
            filter_subs: HashMap::new(),
            resources: HashMap::new(),
            tokens: HashMap::new(),
        }
    }

    pub fn handle_client(&mut self, client: usize, msg: CMessage) -> (Vec<SMessage>, Option<Message>) {
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
                resp.push(SMessage::UpdateResources { serial, resources });
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
                resp.push(SMessage::UpdateResources { serial, resources });
            }
            CMessage::Unsubscribe { serial, ids } => {
                for id in ids {
                    self.subs.get_mut(&id).map(|s| s.remove(&client));
                }
            }
            CMessage::UpdateResource { serial, resource } => {
                match self.resources.get(&resource.id) {
                    Some(&Resource::Torrent(ref t)) => {
                        rmsg = Some(Message::UpdateTorrent(resource));
                    }
                    Some(&Resource::File(ref f)) => {
                        // TODO: Validate other fields(make sure they're not present)
                        if let Some(p) = resource.priority {
                            rmsg = Some(Message::UpdateFile { id: resource.id, torrent_id: f.torrent_id, priority: p });
                        }
                    }
                    Some(_) => {
                        resp.push(SMessage::PermissionDenied(Error {
                            serial: Some(serial),
                            reason: format!("Only torrents and files have mutable fields"),
                        }));
                    }
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
                        rmsg = Some(Message::RemoveTorrent(id));
                    }
                    Some(&Resource::Tracker(ref t)) => {
                        rmsg = Some(Message::RemoveTracker { id, torrent_id: t.id});
                    }
                    Some(&Resource::Peer(ref p)) => {
                        rmsg = Some(Message::RemovePeer { id, torrent_id: p.id});
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
            CMessage::FilterSubscribe { serial, criteria } => {
                let f = Filter { criteria, client };
                let mut ids = Vec::new();
                for (_, r) in self.resources.iter() {
                    if f.matches(r) {
                        ids.push(r.id());
                    }
                }
                resp.push(SMessage::ResourcesExtant { serial, ids });
                self.filter_subs.insert(serial, f);
            }
            CMessage::FilterUnsubscribe {
                serial,
                filter_serial,
            } => {
                self.filter_subs.remove(&filter_serial);
            }

            CMessage::UploadTorrent { serial, size, path } => {
                resp.push(self.new_transfer(
                    serial,
                    TransferKind::UploadTorrent { size, path },
                ));
            }
            CMessage::UploadMagnet { serial, uri, path } => {}
            CMessage::UploadFiles { serial, size, path } => {
                resp.push(self.new_transfer(
                    serial,
                    TransferKind::UploadFiles { size, path },
                ));
            }
            CMessage::DownloadFile { serial, id } => {
                let path = match self.resources.get(&id) {
                    Some(&Resource::File(ref f)) => f.path.clone(),
                    _ => {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown file id {}", id),
                        }));
                        return (resp, rmsg);
                    }
                };
                resp.push(self.new_transfer(
                    serial,
                    TransferKind::DownloadFile { path },
                ));
            }
        }
        (resp, rmsg)
    }

    fn new_transfer(&mut self, serial: u64, kind: TransferKind) -> SMessage {
        let expiration = Utc::now() + Duration::minutes(2);
        let tok = random_string(15);
        self.tokens.insert(
            tok.clone(),
            BearerToken { expiration, kind },
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
        self.criteria.iter().all(|c| r.matches(c))
    }
}
