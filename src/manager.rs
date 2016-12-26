use piece_field::PieceField;

lazy_static! {
    pub static ref PEER_ID: [u8; 20] = {
        use rand::{self, Rng};

        let mut pid = [0u8; 20];
        pid[0] = b'-';
        pid[1] = b'S';
        pid[2] = b'Y';
        pid[3] = b'0';
        pid[4] = b'0';
        pid[5] = b'0';
        pid[6] = b'1';
        pid[7] = b'-';

        let mut rng = rand::thread_rng();
        for i in 8..19 {
            pid[i] = rng.gen::<u8>();
        }
        pid
    };
}

pub struct TorrentData {
    pub pieces: PieceField,
}
