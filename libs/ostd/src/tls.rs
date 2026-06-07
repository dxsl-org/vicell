//! TLS 1.3 client helpers for app cells.
//!
//! Wraps the raw TLS IPC opcodes (0x30–0x32) exposed by the net service cell
//! into ergonomic functions.  All helpers are blocking; they send one IPC
//! message and wait for the reply.
//!
//! ## Usage
//! ```no_run
//! // Look up net service TID at startup.
//! let net = sys_lookup_service(api::service::NET).unwrap();
//!
//! let cap = tls_connect(net, [93, 184, 216, 34], 443, "example.com");
//! if cap == 0 { /* handle error */ }
//!
//! tls_write(net, cap, b"GET / HTTP/1.0\r\n\r\n");
//! let mut resp = [0u8; 512];
//! let n = tls_read(net, cap, &mut resp);
//! tls_close(net, cap);
//! ```

extern crate alloc;

use crate::syscall::{sys_recv, sys_send, SyscallResult};

// TLS IPC opcodes (mirrors cells/services/net/src/poll_driver.rs cell_opcodes).
const TLS_CONNECT: u8 = 0x30;
const TLS_SEND:    u8 = 0x31;
const TLS_RECV:    u8 = 0x32;
const CLOSE:       u8 = 0x15;

/// Open a TLS 1.3 connection to `addr:port` with the given SNI `hostname`.
///
/// Sends `TLS_CONNECT` to the net service at `net_tid`.  Blocks until the TCP
/// handshake and TLS handshake complete, or until the net service times out.
///
/// Returns the `cap_id` (non-zero on success, 0 on failure).
pub fn tls_connect(net_tid: usize, addr: [u8; 4], port: u16, hostname: &str) -> u64 {
    // Wire format: [0x30][cap:8=0][addr:4][port:2 LE][hostname bytes]
    let hn = hostname.as_bytes();
    // Max IPC buf = 512; header = 1+8+4+2 = 15 bytes; hostname = up to 497 bytes.
    let hn_len = hn.len().min(497);
    let msg_len = 15 + hn_len;
    let mut msg = alloc::vec![0u8; msg_len];
    msg[0] = TLS_CONNECT;
    // cap placeholder (bytes 1-8): 0
    msg[9..13].copy_from_slice(&addr);
    msg[13..15].copy_from_slice(&port.to_le_bytes());
    msg[15..15 + hn_len].copy_from_slice(&hn[..hn_len]);

    sys_send(net_tid, &msg);
    let mut reply = [0u8; 8];
    match sys_recv(0, &mut reply) {
        SyscallResult::Ok(_) => u64::from_le_bytes(reply),
        _ => 0,
    }
}

/// Write data to an established TLS connection.
///
/// Returns the number of bytes accepted by the net service (may be less than
/// `data.len()` if the send buffer is temporarily full — retry on partial write).
pub fn tls_write(net_tid: usize, cap_id: u64, data: &[u8]) -> usize {
    // Wire format: [0x31][cap:8 LE][data:*]
    let payload_len = data.len().min(503); // 512 - 9 header bytes
    let msg_len = 9 + payload_len;
    let mut msg = alloc::vec![0u8; msg_len];
    msg[0] = TLS_SEND;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    msg[9..9 + payload_len].copy_from_slice(&data[..payload_len]);

    sys_send(net_tid, &msg);
    let mut reply = [0u8; 4];
    match sys_recv(0, &mut reply) {
        SyscallResult::Ok(_) => u32::from_le_bytes(reply) as usize,
        _ => 0,
    }
}

/// Read decrypted data from an established TLS connection.
///
/// Returns the number of bytes written into `buf`.  Returns 0 if no data is
/// available yet (non-blocking on the server side; the TLS transport itself
/// blocks until the next TLS record arrives, so the call may take a while).
pub fn tls_read(net_tid: usize, cap_id: u64, buf: &mut [u8]) -> usize {
    // Wire format: [0x32][cap:8 LE][buf_len:4 LE]
    let mut msg = [0u8; 13];
    msg[0] = TLS_RECV;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    let want = (buf.len() as u32).to_le_bytes();
    msg[9..13].copy_from_slice(&want);

    sys_send(net_tid, &msg);
    let mut tmp = alloc::vec![0u8; buf.len()];
    match sys_recv(0, &mut tmp) {
        SyscallResult::Ok(_) => {
            // Zero-scan to find actual reply length.
            let n = tmp.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
            let n = n.min(buf.len());
            buf[..n].copy_from_slice(&tmp[..n]);
            n
        }
        _ => 0,
    }
}

/// Close a TLS connection (also removes the underlying TCP socket).
pub fn tls_close(net_tid: usize, cap_id: u64) {
    let mut msg = [0u8; 9];
    msg[0] = CLOSE;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    sys_send(net_tid, &msg);
    let mut r = [0u8; 1];
    let _ = sys_recv(0, &mut r);
}
