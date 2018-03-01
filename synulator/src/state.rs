use std::collections::{HashMap, HashSet};
use std::mem;
use std::borrow::Cow;

use json;
use rand::{self, Rng};

use rpc::message::{CMessage, Error, SMessage};
use rpc::criterion::{self, Criterion, Operation};
use rpc::resource::{self, merge_json, Resource, ResourceKind, SResourceUpdate};

const TORRENTS: usize = 1000;
const FILES: u32 = 140;
const TRACKERS: u8 = 10;
const PEERS: u16 = 150;

pub struct State {
    subs: HashMap<String, HashSet<usize>>,
    filter_subs: HashMap<(usize, u64), Filter>,
    resources: HashMap<String, Resource>,
    // Index by resource kind
    kinds: Vec<HashSet<String>>,
    // Index by torrent ID
    torrent_idx: HashMap<String, HashSet<String>>,
    user_data: HashMap<String, json::Value>,
}

struct Filter {
    _kind: ResourceKind,
    criteria: Vec<Criterion>,
}

impl State {
    pub fn new() -> State {
        let mut s = State {
            subs: HashMap::new(),
            filter_subs: HashMap::new(),
            resources: HashMap::new(),
            torrent_idx: HashMap::new(),
            kinds: vec![HashSet::new(); 6],
            user_data: HashMap::new(),
        };
        s.load();
        s
    }

    pub fn load(&mut self) {
        let mut res = vec![];
        res.push(Resource::Server(resource::Server {
            id: "synulator".to_owned(),
            ..Default::default()
        }));
        let mut rng = rand::thread_rng();
        for i in 0..TORRENTS {
            let peers = rng.gen_range(0, PEERS);
            let trackers = rng.gen_range(0, TRACKERS);
            let files = rng.gen_range(0, FILES);
            let tid = format!("torrent-{}", i);
            res.push(Resource::Torrent(resource::Torrent {
                id: tid.clone(),
                name: Some(tid.clone()),
                status: resource::Status::Idle,
                progress: 1.0,
                peers,
                trackers,
                pieces: Some(1000),
                piece_size: Some(16384),
                transferred_up: rand::random::<u32>() as u64,
                transferred_down: rand::random::<u32>() as u64,
                files: Some(files),
                ..Default::default()
            }));

            for peer in 0..peers {
                res.push(Resource::Peer(resource::Peer {
                    id: format!("torrent-{}-peer-{}", i, peer),
                    torrent_id: tid.clone(),
                    ..Default::default()
                }));
            }

            for tracker in 0..trackers {
                res.push(Resource::Tracker(resource::Tracker {
                    id: format!("torrent-{}-tracker-{}", i, tracker),
                    torrent_id: tid.clone(),
                    ..Default::default()
                }));
            }

            for file in 0..files {
                res.push(Resource::File(resource::File {
                    id: format!("torrent-{}-file-{}", i, file),
                    torrent_id: tid.clone(),
                    path: format!("file-{}-path", file),
                    ..Default::default()
                }));
            }
        }
        self.add_resources(res);
    }

    fn add_resources(&mut self, e: Vec<Resource>) {
        let mut ids = Vec::new();
        for mut r in e {
            ids.push(r.id().to_owned());

            self.subs.insert(r.id().to_owned(), HashSet::default());
            let id = r.id().to_owned();

            self.kinds[r.kind() as usize].insert(id.clone());

            if let Some(tid) = r.torrent_id() {
                if !self.torrent_idx.contains_key(tid) {
                    self.torrent_idx.insert(tid.to_owned(), HashSet::default());
                }
                self.torrent_idx.get_mut(tid).unwrap().insert(id.clone());
            }

            if let Some(user_data) = self.user_data.get(&id) {
                mem::replace(r.user_data(), user_data.clone());
            }
            self.resources.insert(id, r);
        }
    }

    pub fn handle_client(&mut self, client: usize, msg: CMessage) -> Vec<SMessage> {
        let mut resp = Vec::new();
        // let mut rmsg = None;
        match msg {
            CMessage::GetResources { serial, ids } => {
                let mut resources = Vec::new();
                for id in ids {
                    if let Some(r) = self.resources.get(&id) {
                        resources.push(SResourceUpdate::Resource(Cow::Borrowed(r)));
                    } else {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown resource id {}", id),
                        }));
                    }
                }
                resp.push(SMessage::UpdateResources {
                    serial: Some(serial),
                    resources,
                });
            }
            CMessage::Subscribe { serial, ids } => {
                let mut resources = Vec::new();
                for id in ids {
                    if let Some(r) = self.resources.get(&id) {
                        resources.push(SResourceUpdate::Resource(Cow::Borrowed(r)));
                        self.subs.get_mut(&id).map(|s| s.insert(client));
                    } else {
                        resp.push(SMessage::UnknownResource(Error {
                            serial: Some(serial),
                            reason: format!("unknown resource id {}", id),
                        }));
                    }
                }
                resp.push(SMessage::UpdateResources {
                    serial: None,
                    resources,
                });
            }
            CMessage::Unsubscribe { ids, .. } => for id in ids {
                self.subs.get_mut(&id).map(|s| s.remove(&client));
            },
            CMessage::UpdateResource {
                serial,
                mut resource,
            } => {
                let udo = mem::replace(&mut resource.user_data, None);
                if let Some(user_data) = udo {
                    if let Some(res) = self.resources.get_mut(&resource.id) {
                        merge_json(res.user_data(), &mut user_data.clone());
                        self.user_data
                            .insert(res.id().to_owned(), res.user_data().clone());
                        resp.push(SMessage::UpdateResources {
                            serial: Some(serial),
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
            }
            CMessage::RemoveResource {
                serial,
                id,
                ..
            } => match self.resources.get(&id) {
                Some(_) => {
                    resp.push(SMessage::InvalidResource(Error {
                        serial: Some(serial),
                        reason: format!("Resources cannot be removed"),
                    }));
                }
                None => {
                    resp.push(SMessage::UnknownResource(Error {
                        serial: Some(serial),
                        reason: format!("unknown resource id {}", id),
                    }));
                }
            },
            CMessage::FilterSubscribe {
                serial,
                kind,
                criteria,
            } => {
                let torrent_idx = &self.torrent_idx;
                let rkind = &self.kinds[kind as usize];
                let resources = &self.resources;

                let get_matching = |f: &Filter| {
                    let mut added = HashSet::new();
                    let crit_res = f.criteria
                        .iter()
                        .find(|c| c.field == "torrent_id" && c.op == Operation::Eq)
                        .and_then(|c| match &c.value {
                            &criterion::Value::S(ref s) => Some(s),
                            _ => None,
                        })
                        .and_then(|id| torrent_idx.get(id));

                    if let Some(t) = crit_res {
                        println!("fast match");
                        for id in rkind.intersection(t) {
                            let r = resources.get(id).unwrap();
                            if f.matches(r) {
                                added.insert(Cow::Borrowed(r.id()));
                            }
                        }
                    } else {
                        println!("slow match");
                        for id in rkind.iter() {
                            let r = resources.get(id).unwrap();
                            if f.matches(r) {
                                added.insert(Cow::Borrowed(r.id()));
                            }
                        }
                    }
                    added
                };

                let f = Filter { criteria, _kind: kind };
                let matching = get_matching(&f);
                if let Some(prev) = self.filter_subs.insert((client, serial), f) {
                    let prev_matching = get_matching(&prev);
                    let added: Vec<_> = matching.difference(&prev_matching).cloned().collect();
                    let removed: Vec<_> = prev_matching
                        .difference(&matching)
                        .map(Cow::to_string)
                        .collect();

                    if !added.is_empty() {
                        resp.push(SMessage::ResourcesExtant { serial, ids: added });
                    }
                    if !removed.is_empty() {
                        resp.push(SMessage::ResourcesRemoved {
                            serial,
                            ids: removed,
                        });
                    }
                } else {
                    resp.push(SMessage::ResourcesExtant {
                        serial,
                        ids: matching.into_iter().collect(),
                    });
                }
            }
            CMessage::FilterUnsubscribe { filter_serial, .. } => {
                self.filter_subs.remove(&(client, filter_serial));
            }

            CMessage::PauseTorrent { serial, .. } => {
                resp.push(SMessage::UnknownResource(Error {
                    serial: Some(serial),
                    reason: format!("Pause not supported"),
                }));
            }
            CMessage::ResumeTorrent { serial, .. } => {
                resp.push(SMessage::UnknownResource(Error {
                    serial: Some(serial),
                    reason: format!("Resume not supported"),
                }));
            }
            CMessage::AddPeer { serial, .. } => {
                resp.push(SMessage::UnknownResource(Error {
                    serial: Some(serial),
                    reason: format!("Peer add not supported"),
                }));
            }
            CMessage::AddTracker { serial, .. } => {
                resp.push(SMessage::UnknownResource(Error {
                    serial: Some(serial),
                    reason: format!("Tracker add not supported"),
                }));
            }
            CMessage::UpdateTracker { serial, .. } => {
                resp.push(SMessage::InvalidRequest(Error {
                    serial: Some(serial),
                    reason: format!("Tracker update not supported!"),
                }));
            }
            CMessage::ValidateResources { serial, .. } => {
                resp.push(SMessage::InvalidRequest(Error {
                    serial: Some(serial),
                    reason: format!("Validate not supported!"),
                }));
            }
            CMessage::UploadTorrent {
                serial,
                ..
            } => {
                resp.push(SMessage::InvalidRequest(Error {
                    serial: Some(serial),
                    reason: format!("Upload not supported!"),
                }));
            }
            CMessage::UploadMagnet {
                serial,
                ..
            } => {
                resp.push(SMessage::InvalidRequest(Error {
                    serial: Some(serial),
                    reason: format!("Upload not supported!"),
                }));
            }
            CMessage::UploadFiles { serial, .. } => {
                resp.push(SMessage::InvalidRequest(Error {
                    serial: Some(serial),
                    reason: format!("Upload not supported!"),
                }));
            }
        }
        resp
    }

    /*
    fn process_msg(&mut self) {
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
                    msgs.push((
                        c,
                        SMessage::UpdateResources {
                            serial: None,
                            resources,
                        },
                    ));
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
                            ids: ids.into_iter().map(|s| s.into_owned()).collect(),
                        },
                    ));
                }

                for id in r {
                    let r = self.resources.remove(&id).unwrap();
                    if self.user_data.remove(&id).is_some() {
                        self.serialize();
                    }
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
    }

    /// Produces a map of the form Map<(Client ID, Serial), messages)>.
    fn get_matching_filters<'a, I: Iterator<Item = &'a str>>(
        &'a self,
        ids: I,
    ) -> HashMap<(usize, u64), Vec<Cow<'a, str>>> {
        let mut matched = HashMap::new();
        for id in ids {
            let res = self.resources
                .get(id)
                .expect("Bad resource requested from a CtlMessage");
            for (k, f) in self.filter_subs.iter() {
                if f.kind == res.kind() && f.matches(&res) {
                    if !matched.contains_key(k) {
                        matched.insert(k.clone(), Vec::new());
                    }
                    matched.get_mut(&k).unwrap().push(Cow::Borrowed(id));
                }
            }
        }
        matched
    }
    */
}

impl Filter {
    pub fn matches(&self, r: &Resource) -> bool {
        self.criteria.iter().all(|c| c.matches(r))
    }
}
