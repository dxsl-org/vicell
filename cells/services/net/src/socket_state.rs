/// Per-socket connection state for the net service cell.
///
/// Guards against protocol misuse (double-connect, send before connect).
#[allow(dead_code)] // reason: Listening + Closed reserved for LISTEN/ACCEPT server path (Phase B+)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    /// SOCKET_TCP created, no connect attempt yet.
    Created,
    /// CONNECT sent, TCP SYN in flight — smoltcp has not yet reached `Established`.
    Connecting,
    /// TCP handshake complete; SEND/RECV are valid.
    Connected,
    /// LISTEN called; socket is waiting for an inbound SYN.
    Listening,
    /// CLOSE called or RST/FIN received.
    Closed,
}
