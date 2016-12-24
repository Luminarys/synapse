use std::mem;
use std::time::Instant;

enum Interest {
    Interested,
    Uninterested,
}

struct PeerData {
    // Only maintain remote interest, state machine should track our state otherwise
    pub interest: Interest,
    pub received: usize,
    pub last_action: Instant
}

impl PeerData {
    fn new() -> PeerData {
        PeerData {
            interest: Interest::Uninterested,
            received: 0,
            last_action: Instant::now(),
        }
    }
}

pub enum PeerEvent {
    // Initialization
    Init,
    // Got a handshake
    Handshake,
    // We got the piece bitfield
    Bitfield(bool),
    // We received a piece from somewhere else
    HavePiece,
    // We were unchoked
    Unchoked,
    // We received a piece from this peer
    ReceivedPiece,
    // The peer is interested
    Interested,
    // The peer is uninterested
    Uninterested,
    // We were chocked
    Choked,
    // The peer wants a piece
    RequestPiece,

    // We received a piece from somewhere else
    ReceivedExternalPiece,
    // The manager(TBD) deems this a good peer, or optimistically unchoked, and should be allowed
    // to DL
    AllowReciprocation,
    // The manager doesn't want to reciprocate to this peer anymore
    RevokeReciprocation,
    // The torrent was completed
    CompletedTorrent,
    // No pieces can be requested from this peer
    NotRequestable,
    // This connection should be terminated
    Terminate,
}

pub enum PeerReaction {
    SendBF,
    SendInterested,
    SendUninterested,
    SendRequest,
    SendUnchoke,
    SendChoke,
    SendHave,
    SendPiece,
    SendCancel,
    // used in endgame mode, cancel last piece, request a new one
    CancelAndReq,
    Terminate,
    // Instructs the manager to release the piece this was attempting to retrieve, and let another
    // peer handle its retrieval
    ReleasePiece,
    Nothing,
}

enum State {
    Null,
    // Starting state for an incomplete torrent, waiting for events
    Initial,
    // The handshake went through, the peer is valid
    Valid,
    // The peer has nothing to offer us, waiting for HAVE messages
    Uninteresting,
    // The peer has stuff we want, we're waiting for them to unchoke us
    AwaitingUnchoke,
    // We've been unchoked and can now download
    Unchoked,
    // We sent a request and are waiting for a piece back
    AwaitingPiece,
    // We have everything
    Seeding,
}

pub struct Peer {
    can_recip: bool,
    data: PeerData,
    state: State,
}

impl Peer {
    fn new() -> Peer {
        Peer {
            can_recip: false,
            data: PeerData::new(),
            state: State::Initial,
        }
    }

    fn data<'a>(&'a self) -> &'a PeerData {
        &self.data
    }

    // Drive the state machine
    fn handle(&mut self, event: PeerEvent) -> PeerReaction {
        self.data.last_action = Instant::now();
        let old_state = mem::replace(&mut self.state, State::Null);
        let (new_state, resp) = match (old_state, event) {
            (State::Initial, PeerEvent::Handshake) => {
                (State::Valid, PeerReaction::SendBF)
            }
            (State::Valid, PeerEvent::Bitfield(bf)) => {
                // Check if bitfield is interesting - use bool as placeholder
                if bf {
                    // Try to get the peer to unchoke us, manager should priotiize seeding to this
                    // peer
                    (State::AwaitingUnchoke, PeerReaction::SendInterested)
                } else {
                    // Just wait, no need to transition state here
                    (State::Valid, PeerReaction::Nothing)
                }
            }
            (State::AwaitingUnchoke, PeerEvent::Unchoked) => {
                (State::AwaitingPiece, PeerReaction::SendRequest)
            }
            (State::AwaitingPiece, PeerEvent::ReceivedPiece) => {
                self.data.received += 1;
                // If we still have pieces retrievable from this peer, send another request,
                // otherwise send uninterested
                if true {
                    (State::AwaitingPiece, PeerReaction::SendRequest)
                } else {
                    (State::Unchoked, PeerReaction::SendUninterested)
                }
            }
            (State::Unchoked, PeerEvent::Choked) => {
                // If we're choked by an idle peer, just revert to valid
                (State::Valid, PeerReaction::Nothing)
            }
            (State::AwaitingPiece, PeerEvent::Choked) => {
                // If we're choked while waiting for a piece, just wait
                (State::AwaitingUnchoke, PeerReaction::ReleasePiece)
            }
            (State::Unchoked, PeerEvent::HavePiece) => {
                // If the peer got a piece we want and isn't choking us request it
                if true {
                    (State::AwaitingPiece, PeerReaction::SendRequest)
                } else {
                    (State::Unchoked, PeerReaction::Nothing)
                }
            }
            (State::Valid, PeerEvent::HavePiece) => {
                // If the peer got a piece we want and isn't choking us request it
                if true {
                    (State::AwaitingUnchoke, PeerReaction::SendInterested)
                } else {
                    (State::Valid, PeerReaction::Nothing)
                }
            }
            (s, PeerEvent::HavePiece) => {
                // Just modify state so we know we got the piece
                (s, PeerReaction::Nothing)
            }
            (State::AwaitingPiece, PeerEvent::ReceivedExternalPiece) => {
                // If this is a piece we want rn cancel and req, otherwise announce have
                let s = State::AwaitingPiece;
                if true {
                    (s, PeerReaction::CancelAndReq)
                } else {
                    // If the peer doens't have this piece inform them
                    if true {
                        (s, PeerReaction::SendHave)
                    } else {
                        (s, PeerReaction::Nothing)
                    }
                }
            }
            (s, PeerEvent::AllowReciprocation) => {
                self.can_recip = true;
                if let Interest::Interested = self.data.interest {
                    (s, PeerReaction::SendUnchoke)
                } else {
                    (s, PeerReaction::Nothing)
                }
            }
            (s, PeerEvent::RevokeReciprocation) => {
                self.can_recip = false;
                if let Interest::Interested = self.data.interest {
                    (s, PeerReaction::SendChoke)
                } else {
                    (s, PeerReaction::Nothing)
                }
            }
            (State::Seeding, PeerEvent::RequestPiece) => {
                (State::Seeding, PeerReaction::SendPiece)
            }
            (s, PeerEvent::RequestPiece) => {
                if self.can_recip {
                    (s, PeerReaction::SendPiece)
                } else {
                    // Peers should not be requesting when we have choked them, kill conn
                    (s, PeerReaction::Terminate)
                }
            }
            (s, PeerEvent::Interested) => {
                self.data.interest = Interest::Interested;
                (s, PeerReaction::Nothing)
            }
            (State::Seeding, PeerEvent::Uninterested) => {
                // If we're seeding and for some reason the peer is uninterested, terminate
                // connection
                self.data.interest = Interest::Uninterested;
                (State::Seeding, PeerReaction::Terminate)
            }
            (s, PeerEvent::Uninterested) => {
                self.data.interest = Interest::Uninterested;
                (s, PeerReaction::Nothing)
            }
            (s, PeerEvent::ReceivedExternalPiece) => {
                // If we got a piece from somewhere else, and they don't have it inform this peer
                if true {
                    (s, PeerReaction::SendHave)
                } else {
                    (s, PeerReaction::Nothing)
                }
            }
            (State::AwaitingPiece, PeerEvent::CompletedTorrent) => {
                (State::Seeding, PeerReaction::SendCancel)
            }
            (_, PeerEvent::CompletedTorrent) => {
                (State::Seeding, PeerReaction::Nothing)
            }
            (state, _event) => {
                (state, PeerReaction::Nothing)
            }
        };
        self.state = new_state;
        resp
    }
}
