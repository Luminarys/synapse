enum Interest {
    Interested,
    Uninterested,
}

enum Choke {
    Choked,
    Unchoked,
}

struct PeerData {
    // Only maintain remote choke/interest, state machine should track our state
    choke: Choke,
    interest: Interest,
}

pub enum PeerEvent {
    // Got a handshake
    Handshake,
    // We got the piece bitfield
    Bitfield,
    // We received a piece from somewhere else
    HavePiece,
    // We were unchoked
    Unchoked,
    // We received a piece from this peer
    ReceivedPiece,
    // The peer is interested
    Interested,
    // The peer is uninterested
    UnInterested,
    // We were chocked
    Choked,
    // The peer wants a piece
    RequestPiece,

    // Something timed out
    Timeout,
    // We received a piece from somewhere else
    ReceivedExternalPiece,
    // The manager(TBD) deems this a good peer, or optimistically unchoked, and should be allowed
    // to DL
    AllowReciprocation,
    // The manager doesn't want to reciprocate to this peer anymore
    RevokeReciprocation,
    // The torrent was completed
    CompletedTorrent,
    // This connection should be terminated
    Terminate,
}

pub enum PeerReaction {
    StartHandshake,
    GetBF,
    SendInterest,
    SendRequest,
    SendUnchoke,
    SendChoke,
    SendHave,
    SendCancel,
    SendPing,
    BroadcastPiece,
    Terminate,
}

enum State {
    // Starting state for an incomplete torrent, waiting for events
    Initial,
    // The handshake went through, the peer is valid
    Valid,
    // The peer has nothing to offer us, waiting for HAVE messages
    Uninteresting,
    // The peer has stuff we want, we're waiting for them to unchoke us
    WaitingUnchoke,
    // We've been unchoked and can now download
    Unchoked,
    // We sent a request and are waiting for a piece back
    AwaitingPiece,
}

pub struct Peer {
    data: PeerData,
    state: State,
}
