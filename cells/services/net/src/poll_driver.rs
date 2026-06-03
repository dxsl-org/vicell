//! Net cell polling driver — merges timer ticks with IPC messages into
//! a unified smoltcp.poll() call cadence.
//!
//! The net cell must call smoltcp at least every `POLL_INTERVAL_MS` ms to
//! drive TCP retransmissions, DHCP renewal, and ARP refreshes.  It also
//! processes inbound IPC messages from both the kernel (raw frames) and
//! consumer cells (socket syscalls).

/// Recommended poll interval in milliseconds.
pub const POLL_INTERVAL_MS: u64 = 100;

/// IPC opcodes the net cell recognises from the kernel VirtIO driver.
pub mod kernel_opcodes {
    /// Kernel sends a raw Ethernet frame to be processed by smoltcp.
    pub const RX_FRAME: u8 = 0x00;
    /// Kernel signals that TX queue has drained (not strictly required).
    pub const TX_DONE: u8  = 0x02;
}

/// IPC opcodes the net cell recognises from consumer cells.
pub mod cell_opcodes {
    /// Create a TCP socket; reply = CapId.
    pub const SOCKET_TCP: u8    = 0x10;
    /// Create a UDP socket; reply = CapId.
    pub const SOCKET_UDP: u8    = 0x11;
    /// Connect TCP socket to (addr4[4], port[2]); reply = 0|1.
    pub const CONNECT: u8       = 0x12;
    /// Send data on a TCP/UDP socket.
    pub const SEND: u8          = 0x13;
    /// Recv data from a TCP/UDP socket; reply = bytes.
    pub const RECV: u8          = 0x14;
    /// Close socket; reply = 0.
    pub const CLOSE: u8         = 0x15;
    /// Bind UDP socket to local port.
    pub const BIND: u8          = 0x16;
    /// Listen for TCP connections on a port; reply = CapId for accept.
    pub const LISTEN: u8        = 0x17;
    /// Accept a pending TCP connection; reply = CapId for new stream.
    pub const ACCEPT: u8        = 0x18;
    /// Query live TCP state of a socket; reply = 1-byte state code.
    pub const SOCKET_STATE: u8  = 0x19;
    /// Query DHCP-assigned IP; reply = 4-byte addr or 0.0.0.0.
    pub const GET_LOCAL_IP: u8  = 0x20;
    /// Send a UDP datagram to (addr4[4], port[2 LE]); reply = bytes queued [4 LE].
    pub const SENDTO: u8        = 0x21;
    /// Receive one UDP datagram; reply = [src_addr:4][src_port:2 LE][data] or empty.
    pub const RECVFROM: u8      = 0x22;
}

/// Decode a raw IPC payload into either a kernel frame event or a cell request.
#[derive(Debug)]
pub enum NetMessage<'a> {
    /// An inbound Ethernet frame from the kernel VirtIO driver.
    RxFrame(&'a [u8]),
    /// A socket syscall from a consumer cell.
    CellRequest { opcode: u8, cap: u64, payload: &'a [u8] },
    /// Unknown or malformed message.
    Unknown,
}

/// Parse an IPC message buffer into a `NetMessage`.
pub fn decode_message(buf: &[u8]) -> NetMessage<'_> {
    if buf.is_empty() { return NetMessage::Unknown; }
    match buf[0] {
        kernel_opcodes::RX_FRAME => {
            NetMessage::RxFrame(&buf[1..])
        }
        kernel_opcodes::TX_DONE => NetMessage::Unknown,
        opcode => {
            // Consumer cell format: [opcode:1][cap:8][payload:*]
            if buf.len() < 9 { return NetMessage::Unknown; }
            let cap = u64::from_le_bytes([
                buf[1], buf[2], buf[3], buf[4],
                buf[5], buf[6], buf[7], buf[8],
            ]);
            NetMessage::CellRequest { opcode, cap, payload: &buf[9..] }
        }
    }
}
