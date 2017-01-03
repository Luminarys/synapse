use piece_field::PieceField;
// use tokio_core::io::{Codec, EasyBuf};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Write};
//
pub enum Message {
    Handshake([u8; 8], [u8; 20], [u8; 20]),
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    Uninterested,
    Have(u32),
    Bitfield(PieceField),
    Request(u32, u32, u32),
    Piece(u32, u32, Box<[u8]>),
    Cancel(u32, u32, u32),
}

impl Message {
    fn encode(self, mut buf: &mut [u8]) -> io::Result<()> {
        match self {
            Message::Handshake(reserved, hash, pid) => {
                if pid.len() != 20 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Peer ID"));
                }
                buf.write_u8(19)?;
                buf.write_all("BitTorrent protocol".as_ref())?;
                buf.write_all(&reserved)?;
                buf.write_all(&hash)?;
                buf.write_all(&pid)?;
            }
            Message::KeepAlive => {
                buf.write_u32::<BigEndian>(0)?;
            }
            Message::Choke => {
                buf.write_u32::<BigEndian>(1)?;
                buf.write_u8(0)?;
            }
            Message::Unchoke => {
                buf.write_u32::<BigEndian>(1)?;
                buf.write_u8(1)?;
            }
            Message::Interested => {
                buf.write_u32::<BigEndian>(1)?;
                buf.write_u8(2)?;
            }
            Message::Uninterested => {
                buf.write_u32::<BigEndian>(1)?;
                buf.write_u8(3)?;
            }
            Message::Have(piece) => {
                buf.write_u32::<BigEndian>(5)?;
                buf.write_u8(4)?;
                buf.write_u32::<BigEndian>(piece)?;
            }
            Message::Bitfield(pf) => {
                let (data, _) = pf.extract();
                buf.write_u32::<BigEndian>(1 + data.len() as u32)?;
                buf.write_u8(5)?;
                for i in 0..data.len() {
                    buf.write_u8(data[i])?;
                }
            }
            Message::Request(index, begin, length) => {
                buf.write_u32::<BigEndian>(13)?;
                buf.write_u8(6)?;
                buf.write_u32::<BigEndian>(index)?;
                buf.write_u32::<BigEndian>(begin)?;
                buf.write_u32::<BigEndian>(length)?;
            }
            Message::Piece(index, begin, data) => {
                buf.write_u32::<BigEndian>(9 + data.len() as u32)?;
                buf.write_u8(7)?;
                buf.write_u32::<BigEndian>(index)?;
                buf.write_u32::<BigEndian>(begin)?;
                // This may be inefficient, as it's an extra copy.
                buf.write_all(&data)?;
            }
            Message::Cancel(index, begin, length) => {
                buf.write_u32::<BigEndian>(13)?;
                buf.write_u8(8)?;
                buf.write_u32::<BigEndian>(index)?;
                buf.write_u32::<BigEndian>(begin)?;
                buf.write_u32::<BigEndian>(length)?;
            }
        };
        Ok(())
    }
}
//
// pub struct MessageCodec {
//     hash: [u8; 20],
//     pieces: u32,
//     got_handshake: bool,
//     len: Option<u32>,
// }
//
// impl Codec for MessageCodec {
//     type In = Message;
//     type Out = Message;
//
//     fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<Message>, io::Error> {
//         let len = match (self.len, buf.len()) {
//             (Some(l), _) => l,
//             (None, l) => {
//                 if l < 4 {
//                     return Ok(None);
//                 }
//                 let len = buf.drain_to(4).as_slice().read_u32::<BigEndian>()?;
//                 self.len = Some(len);
//                 len
//             }
//         };
//
//         if !self.got_handshake {
//             // If we wanted to follow the spec 100% we'd try to get hash ID before peer ID,
//             // but these days connections are so fast it should not matter
//             if 49 + len as usize > buf.len() {
//                 return Ok(None);
//             }
//             if buf.drain_to(len as usize).as_slice() != b"BitTorrent protocol" {
//                 return Err(io::Error::new(io::ErrorKind::InvalidData, "Handshake must be for the BitTorrent protocol"));
//             }
//             let mut reserved = [0u8; 8];
//             reserved.iter_mut().zip(buf.drain_to(8).as_slice()).map(|(v, c)| {
//                 *v = *c;
//             }).last();
//             let mut hash = [0u8; 20];
//             hash.iter_mut().zip(buf.drain_to(20).as_slice()).map(|(v, c)| {
//                 *v = *c;
//             }).last();
//             if hash != self.hash {
//                 return Err(io::Error::new(io::ErrorKind::InvalidData, "Hashes must match!"));
//             }
//             let mut pid = [0u8; 20];
//             pid.iter_mut().zip(buf.drain_to(20).as_slice()).map(|(v, c)| {
//                 *v = *c;
//             }).last();
//             self.got_handshake = true;
//             return Ok(Some(Message::Handshake(reserved, hash, pid)));
//         }
//
//
//         if len as usize > buf.len() {
//             return Ok(None);
//         }
//
//         self.len = None;
//
//         if len == 0 {
//             return Ok(Some(Message::KeepAlive));
//         }
//
//         // Drain out the necessary portion for the frame, use that
//         let mut mbuf = buf.drain_to(len as usize);
//
//         let id = mbuf.drain_to(1).as_slice().read_u8()?;
//
//         match id {
//             0 => Ok(Some(Message::Choke)),
//             1 => Ok(Some(Message::Unchoke)),
//             2 => Ok(Some(Message::Interested)),
//             3 => Ok(Some(Message::Uninterested)),
//             4 => {
//                 if mbuf.len() != 4 {
//                     return Err(io::Error::new(io::ErrorKind::InvalidData, "Have message must have length 5"));
//                 }
//                 Ok(Some(Message::Have(mbuf.as_slice().read_u32::<BigEndian>()?)))
//             }
//             5 => {
//                 if mbuf.len() != ((self.pieces - 1)/8 + 1) as usize {
//                     return Err(io::Error::new(io::ErrorKind::InvalidData, "Bitfield message must have length according to the pieces in the torrent"));
//                 }
//                 let mut mmb = mbuf.get_mut();
//                 let pf = PieceField::from(mmb.drain(0..).collect::<Vec<u8>>().into_boxed_slice(), self.pieces);
//                 Ok(Some(Message::Bitfield(pf)))
//             }
//             6 | 8 => {
//                 if mbuf.len() != 12 {
//                     return Err(io::Error::new(io::ErrorKind::InvalidData, "Request/Cancel message must have length 13"));
//                 }
//                 let index = mbuf.drain_to(4).as_slice().read_u32::<BigEndian>()?;
//                 let begin = mbuf.drain_to(4).as_slice().read_u32::<BigEndian>()?;
//                 let length = mbuf.drain_to(4).as_slice().read_u32::<BigEndian>()?;
//                 if id == 6 {
//                     Ok(Some(Message::Request(index, begin, length)))
//                 } else {
//                     Ok(Some(Message::Cancel(index, begin, length)))
//                 }
//             }
//             7 => {
//                 if mbuf.len() < 8 {
//                     return Err(io::Error::new(io::ErrorKind::InvalidData, "Piece message must have length at least 9"));
//                 }
//                 let index = mbuf.drain_to(4).as_slice().read_u32::<BigEndian>()?;
//                 let begin = mbuf.drain_to(4).as_slice().read_u32::<BigEndian>()?;
//                 let mut mmb = mbuf.get_mut();
//                 let data = mmb.drain(0..).collect::<Vec<u8>>().into_boxed_slice();
//                 Ok(Some(Message::Piece(index, begin, data)))
//             }
//             _ => {
//                 Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid ID"))
//             }
//         }
//     }
//
//     fn encode(&mut self, msg: Message, buf: &mut Vec<u8>) -> io::Result<()> {
//         match msg {
//             Message::Handshake(reserved, hash, pid) => {
//                 if pid.len() != 20 {
//                     return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Peer ID"));
//                 }
//                 buf.write_u8(19)?;
//                 buf.write_all("BitTorrent protocol".as_ref())?;
//                 buf.write_all(&reserved)?;
//                 buf.write_all(&hash)?;
//                 buf.write_all(&pid)?;
//             }
//             Message::KeepAlive => {
//                 buf.write_u32::<BigEndian>(0)?;
//             }
//             Message::Choke => {
//                 buf.write_u32::<BigEndian>(1)?;
//                 buf.write_u8(0)?;
//             }
//             Message::Unchoke => {
//                 buf.write_u32::<BigEndian>(1)?;
//                 buf.write_u8(1)?;
//             }
//             Message::Interested => {
//                 buf.write_u32::<BigEndian>(1)?;
//                 buf.write_u8(2)?;
//             }
//             Message::Uninterested => {
//                 buf.write_u32::<BigEndian>(1)?;
//                 buf.write_u8(3)?;
//             }
//             Message::Have(piece) => {
//                 buf.write_u32::<BigEndian>(5)?;
//                 buf.write_u8(4)?;
//                 buf.write_u32::<BigEndian>(piece)?;
//             }
//             Message::Bitfield(pf) => {
//                 let (data, _) = pf.extract();
//                 buf.write_u32::<BigEndian>(1 + data.len() as u32)?;
//                 buf.write_u8(5)?;
//                 for i in 0..data.len() {
//                     buf.write_u8(data[i])?;
//                 }
//             }
//             Message::Request(index, begin, length) => {
//                 buf.write_u32::<BigEndian>(13)?;
//                 buf.write_u8(6)?;
//                 buf.write_u32::<BigEndian>(index)?;
//                 buf.write_u32::<BigEndian>(begin)?;
//                 buf.write_u32::<BigEndian>(length)?;
//             }
//             Message::Piece(index, begin, data) => {
//                 buf.write_u32::<BigEndian>(9 + data.len() as u32)?;
//                 buf.write_u8(7)?;
//                 buf.write_u32::<BigEndian>(index)?;
//                 buf.write_u32::<BigEndian>(begin)?;
//                 // This may be inefficient, as it's an extra copy.
//                 buf.write_all(&data)?;
//             }
//             Message::Cancel(index, begin, length) => {
//                 buf.write_u32::<BigEndian>(13)?;
//                 buf.write_u8(8)?;
//                 buf.write_u32::<BigEndian>(index)?;
//                 buf.write_u32::<BigEndian>(begin)?;
//                 buf.write_u32::<BigEndian>(length)?;
//             }
//         };
//         Ok(())
//     }
// }
