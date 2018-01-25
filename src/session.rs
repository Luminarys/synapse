pub mod torrent {
    pub use self::ver_6e27af as current;
    pub use self::current::Session;
    use bincode;

    pub fn load(data: &[u8]) -> Option<Session> {
        if let Ok(m) = bincode::deserialize::<ver_6e27af::Session>(data) {
            Some(m)
        } else if let Ok(m) = bincode::deserialize::<ver_249b1b::Session>(data) {
            info!("Migrating torrent session from v249b1b");
            Some(m.migrate())
        } else if let Ok(m) = bincode::deserialize::<ver_5f166d::Session>(data) {
            info!("Migrating torrent session from v5f166d");
            Some(m.migrate())
        } else if let Ok(m) = bincode::deserialize::<ver_8e1121::Session>(data) {
            info!("Migrating torrent session from v8e1121");
            Some(m.migrate())
        } else {
            None
        }
    }

    impl Session {
        pub fn migrate(self) -> Self {
            self
        }
    }

    pub mod ver_6e27af {
        use torrent::Bitfield;

        use chrono::{DateTime, Utc};

        use std::path::PathBuf;

        #[derive(Serialize, Deserialize)]
        pub struct Session {
            pub info: Info,
            pub pieces: Bitfield,
            pub uploaded: u64,
            pub downloaded: u64,
            pub status: Status,
            pub path: Option<String>,
            pub priority: u8,
            pub priorities: Vec<u8>,
            pub created: DateTime<Utc>,
            pub throttle_ul: Option<i64>,
            pub throttle_dl: Option<i64>,
            pub trackers: Vec<String>,
        }

        #[derive(Clone, Serialize, Deserialize)]
        pub struct Info {
            pub name: String,
            pub announce: Option<String>,
            pub piece_len: u32,
            pub total_len: u64,
            pub hashes: Vec<Vec<u8>>,
            pub hash: [u8; 20],
            pub files: Vec<File>,
            pub private: bool,
            pub be_name: Option<Vec<u8>>,
            pub piece_idx: Vec<(usize, u64)>,
        }

        #[derive(Serialize, Deserialize, Clone, Debug)]
        pub struct File {
            pub path: PathBuf,
            pub length: u64,
        }

        #[derive(Clone, Debug, Serialize, Deserialize)]
        pub struct Status {
            pub paused: bool,
            pub validating: bool,
            pub error: Option<String>,
            pub state: StatusState,
        }

        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        pub enum StatusState {
            Magnet,
            // Torrent has not acquired all pieces
            Incomplete,
            // Torrent has acquired all pieces, regardless of validity
            Complete,
        }
    }

    pub mod ver_249b1b {
        pub use super::ver_6e27af as next;
        pub use self::next::{File, Info, Status, StatusState};

        use torrent::Bitfield;

        use chrono::{DateTime, Utc};

        #[derive(Serialize, Deserialize)]
        pub struct Session {
            pub info: Info,
            pub pieces: Bitfield,
            pub uploaded: u64,
            pub downloaded: u64,
            pub status: Status,
            pub path: Option<String>,
            pub priority: u8,
            pub priorities: Vec<u8>,
            pub created: DateTime<Utc>,
            pub throttle_ul: Option<i64>,
            pub throttle_dl: Option<i64>,
        }

        impl Session {
            pub fn migrate(self) -> super::current::Session {
                let mut trackers = Vec::new();
                if let Some(ref url) = self.info.announce {
                    trackers.push(url.to_owned());
                }
                next::Session {
                    info: self.info,
                    pieces: self.pieces,
                    uploaded: self.uploaded,
                    downloaded: self.downloaded,
                    status: self.status,
                    path: self.path,
                    priority: self.priority,
                    priorities: self.priorities,
                    created: self.created,
                    throttle_ul: self.throttle_ul,
                    throttle_dl: self.throttle_dl,
                    trackers,
                }.migrate()
            }
        }
    }

    pub mod ver_5f166d {
        use super::ver_249b1b as next;

        use torrent::Bitfield;

        use chrono::{DateTime, Utc};

        #[derive(Serialize, Deserialize)]
        pub struct Session {
            pub info: Info,
            pub pieces: Bitfield,
            pub uploaded: u64,
            pub downloaded: u64,
            pub status: Status,
            pub path: Option<String>,
            pub priority: u8,
            pub priorities: Vec<u8>,
            pub created: DateTime<Utc>,
            pub throttle_ul: Option<i64>,
            pub throttle_dl: Option<i64>,
        }

        #[derive(Serialize, Deserialize)]
        pub enum Status {
            Pending,
            Paused,
            Leeching,
            Idle,
            Seeding,
            Validating,
            Magnet,
            DiskError,
        }

        #[derive(Serialize, Deserialize)]
        pub struct Info {
            pub name: String,
            pub announce: String,
            pub piece_len: u32,
            pub total_len: u64,
            pub hashes: Vec<Vec<u8>>,
            pub hash: [u8; 20],
            pub files: Vec<next::File>,
            pub private: bool,
            pub be_name: Option<Vec<u8>>,
        }

        impl Session {
            pub fn migrate(self) -> super::current::Session {
                let state = if self.pieces.complete() {
                    next::StatusState::Complete
                } else {
                    next::StatusState::Incomplete
                };
                let paused = match self.status {
                    Status::Paused => true,
                    _ => false,
                };
                let piece_idx = generate_piece_idx(
                    self.info.hashes.len(),
                    self.info.piece_len as u64,
                    &self.info.files,
                );
                next::Session {
                    info: next::Info {
                        name: self.info.name,
                        announce: if self.info.announce == "" {
                            None
                        } else {
                            Some(self.info.announce)
                        },
                        piece_len: self.info.piece_len,
                        total_len: self.info.total_len,
                        hashes: self.info.hashes,
                        hash: self.info.hash,
                        files: self.info.files,
                        private: self.info.private,
                        be_name: self.info.be_name,
                        piece_idx,
                    },
                    pieces: self.pieces,
                    uploaded: self.uploaded,
                    downloaded: self.downloaded,
                    status: next::Status {
                        paused,
                        state,
                        validating: false,
                        error: None,
                    },
                    path: self.path,
                    priority: self.priority,
                    priorities: self.priorities,
                    created: self.created,
                    throttle_ul: self.throttle_ul,
                    throttle_dl: self.throttle_dl,
                }.migrate()
            }
        }

        fn generate_piece_idx(pieces: usize, pl: u64, files: &[next::File]) -> Vec<(usize, u64)> {
            let mut piece_idx = Vec::with_capacity(pieces);
            let mut file = 0;
            let mut offset = 0u64;
            for _ in 0..pieces {
                piece_idx.push((file, offset));
                offset += pl;
                while file < files.len() && offset >= files[file].length {
                    offset -= files[file].length;
                    file += 1;
                }
            }
            piece_idx
        }
    }

    pub mod ver_8e1121 {
        use super::ver_5f166d as next;
        use self::next::{Info, Status};

        use torrent::Bitfield;

        use chrono::{DateTime, Utc};

        #[derive(Serialize, Deserialize)]
        pub struct Session {
            pub info: Info,
            pub pieces: Bitfield,
            pub uploaded: u64,
            pub downloaded: u64,
            pub status: Status,
            pub path: Option<String>,
            pub wanted: Bitfield,
            pub priority: u8,
            pub priorities: Vec<u8>,
            pub created: DateTime<Utc>,
            pub throttle_ul: Option<i64>,
            pub throttle_dl: Option<i64>,
        }

        impl Session {
            pub fn migrate(self) -> super::current::Session {
                next::Session {
                    info: self.info,
                    pieces: self.pieces,
                    uploaded: self.uploaded,
                    downloaded: self.downloaded,
                    status: self.status,
                    path: self.path,
                    priority: self.priority,
                    priorities: self.priorities,
                    created: self.created,
                    throttle_ul: self.throttle_ul,
                    throttle_dl: self.throttle_dl,
                }.migrate()
            }
        }
    }
}
