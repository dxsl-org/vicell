//! httpd — minimal HTTP/1.0 file server for ViOS.
//!
//! Usage: httpd <port> <vfs_path>
//!
//! Listens for TCP connections on <port>.  For each connection, reads the
//! HTTP request (discards it), reads <vfs_path> from the VFS cell via OP_READ
//! IPC, and responds with HTTP/1.0 200 OK + the file content.  Loops forever,
//! serving one connection at a time.

#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::{print, println};
use ostd::syscall::{sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};

const NET_ENDPOINT: usize = 6;
const VFS_ENDPOINT: usize = 3;

const SOCKET_TCP: u8 = 0x10;
const SEND_OP:    u8 = 0x13;
const RECV_OP:    u8 = 0x14;
const CLOSE_OP:   u8 = 0x15;
const LISTEN_OP:  u8 = 0x17;
const ACCEPT_OP:  u8 = 0x18;
const STATE_OP:   u8 = 0x19;
const OP_READ:    u8 = 0x08;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_u16(s: &str) -> Option<u16> {
    let mut n: u32 = 0;
    if s.is_empty() { return None; }
    for ch in s.bytes() {
        if !(b'0'..=b'9').contains(&ch) { return None; }
        n = n * 10 + (ch - b'0') as u32;
        if n > 65535 { return None; }
    }
    Some(n as u16)
}

fn close_cap(cap: u64) {
    let mut msg = [0u8; 9];
    msg[0] = CLOSE_OP;
    msg[1..9].copy_from_slice(&cap.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut r = [0u8; 1];
    let _ = sys_recv(0, &mut r);
}

/// Query TCP socket state — 0x00 = closed, 0x06 = CloseWait.
fn query_state(cap: u64) -> u8 {
    let mut msg = [0u8; 9];
    msg[0] = STATE_OP;
    msg[1..9].copy_from_slice(&cap.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut r = [0u8; 1];
    match sys_recv(0, &mut r) {
        SyscallResult::Ok(_) => r[0],
        _ => 0,
    }
}

/// Read a VFS file via OP_READ IPC.  Returns byte count written into `buf`.
fn vfs_read(path: &str, buf: &mut [u8]) -> usize {
    let pb = path.as_bytes();
    let pl = pb.len().min(253) as u8;
    let mut req = [0u8; 256];
    req[0] = OP_READ;
    req[1] = pl;
    req[2..2 + pl as usize].copy_from_slice(&pb[..pl as usize]);
    sys_send(VFS_ENDPOINT, &req[..2 + pl as usize]);
    buf.fill(0);
    match sys_recv(0, buf) {
        SyscallResult::Ok(_) =>
            buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0),
        _ => 0,
    }
}

/// Send bytes to a TCP socket cap, retrying until all bytes are buffered.
fn tcp_send(cap: u64, data: &[u8]) {
    let mut sent = 0usize;
    for _ in 0..1000 {
        if sent >= data.len() { break; }
        let rem = &data[sent..];
        let chunk = rem.len().min(480);
        let mut msg = [0u8; 9 + 480];
        msg[0] = SEND_OP;
        msg[1..9].copy_from_slice(&cap.to_le_bytes());
        msg[9..9 + chunk].copy_from_slice(&rem[..chunk]);
        sys_send(NET_ENDPOINT, &msg[..9 + chunk]);
        let mut cnt = [0u8; 4];
        match sys_recv(0, &mut cnt) {
            SyscallResult::Ok(_) => {
                let n = u32::from_le_bytes(cnt) as usize;
                sent += n;
                if n == 0 { sys_yield(); }
            }
            _ => break,
        }
    }
}

/// Drain the HTTP request until the header terminator `\r\n\r\n` is seen.
///
/// Returns when the terminator is found, the peer closes, or the retry limit
/// is exceeded.  The request body (if any) is ignored — httpd only serves GET.
fn drain_request(cap: u64) {
    let mut req_msg = [0u8; 13];
    req_msg[0] = RECV_OP;
    req_msg[1..9].copy_from_slice(&cap.to_le_bytes());
    req_msg[9..13].copy_from_slice(&256u32.to_le_bytes());

    let mut buf = [0u8; 256];
    let mut seen_end = false;

    for _ in 0..200 {
        buf.fill(0);
        sys_send(NET_ENDPOINT, &req_msg);
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(_) if buf[0] != 0 => {
                // Check for the HTTP header terminator.
                let end = buf.iter().position(|&b| b == 0).unwrap_or(256);
                let hay = &buf[..end];
                if hay.windows(4).any(|w| w == b"\r\n\r\n") {
                    seen_end = true;
                    break;
                }
            }
            SyscallResult::Ok(_) => {
                let st = query_state(cap);
                if st == 0x06 || st == 0x00 { break; }
                sys_yield();
            }
            _ => break,
        }
        if seen_end { break; }
    }
}

/// Write "Content-Length: N\r\n" as ASCII into `out`.  Returns byte count.
fn write_content_length(n: usize, out: &mut [u8]) -> usize {
    let prefix = b"Content-Length: ";
    let mut pos = 0usize;
    out[pos..pos + prefix.len()].copy_from_slice(prefix);
    pos += prefix.len();
    // ASCII-encode n.
    let mut tmp = [0u8; 20];
    let mut di = 20;
    let mut v = n;
    if v == 0 { tmp[19] = b'0'; di = 19; }
    while v > 0 { di -= 1; tmp[di] = b'0' + (v % 10) as u8; v /= 10; }
    let digits = &tmp[di..];
    out[pos..pos + digits.len()].copy_from_slice(digits);
    pos += digits.len();
    out[pos..pos + 2].copy_from_slice(b"\r\n");
    pos + 2
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[no_mangle]
pub fn main() {
    // Read spawn_args immediately to avoid ARGV_STASH_KEY race.
    let mut arg_buf = [0u8; 128];
    let arg_len = sys_spawn_args(&mut arg_buf);
    if arg_len == 0 {
        println("Usage: httpd <port> <vfs_path>");
        return;
    }
    let args_str = match core::str::from_utf8(&arg_buf[..arg_len]) {
        Ok(s) => s,
        Err(_) => { println("httpd: bad args"); return; }
    };
    let mut parts = args_str.split_whitespace();
    let port = match parts.next().and_then(parse_u16) {
        Some(p) => p,
        None => { println("Usage: httpd <port> <vfs_path>"); return; }
    };
    let path = match parts.next() {
        Some(p) => p,
        None => { println("Usage: httpd <port> <vfs_path>"); return; }
    };

    // Read the file content from VFS once (cache it for subsequent requests).
    let mut file_buf = [0u8; 4096];
    let file_len = vfs_read(path, &mut file_buf);
    if file_len == 0 {
        print("httpd: cannot read '");
        print(path);
        println("'");
        return;
    }

    // Open the listening socket.
    let sock_msg = [SOCKET_TCP, 0, 0, 0, 0, 0, 0, 0, 0];
    sys_send(NET_ENDPOINT, &sock_msg);
    let mut cap_reply = [0u8; 8];
    let listen_cap = match sys_recv(0, &mut cap_reply) {
        SyscallResult::Ok(_) => u64::from_le_bytes(cap_reply),
        _ => { println("httpd: socket failed"); return; }
    };
    if listen_cap == 0 { println("httpd: no socket cap"); return; }

    // LISTEN [0x17][cap:8][port:2 LE]
    let mut listen_msg = [0u8; 11];
    listen_msg[0] = LISTEN_OP;
    listen_msg[1..9].copy_from_slice(&listen_cap.to_le_bytes());
    listen_msg[9..11].copy_from_slice(&port.to_le_bytes());
    sys_send(NET_ENDPOINT, &listen_msg);
    let mut ack = [0u8; 1];
    match sys_recv(0, &mut ack) {
        SyscallResult::Ok(_) if ack[0] == 0x00 => {}
        _ => { println("httpd: listen failed"); close_cap(listen_cap); return; }
    }
    print("httpd: listening on ");
    ostd::io::print_usize(port as usize);
    println("");

    // Accept loop — serve one connection at a time.
    let mut accept_msg = [0u8; 9];
    accept_msg[0] = ACCEPT_OP;
    accept_msg[1..9].copy_from_slice(&listen_cap.to_le_bytes());

    loop {
        // Poll ACCEPT until a connection arrives.
        let stream_cap: u64 = loop {
            sys_send(NET_ENDPOINT, &accept_msg);
            let mut r = [0u8; 8];
            match sys_recv(0, &mut r) {
                SyscallResult::Ok(_) => {
                    let c = u64::from_le_bytes(r);
                    if c != u64::MAX && c != 0 { break c; }
                    sys_yield();
                }
                _ => { sys_yield(); }
            }
        };

        // Drain HTTP request (ignore content — we always serve the same file).
        drain_request(stream_cap);

        // Build and send HTTP/1.0 200 OK response.
        let mut header = [0u8; 128];
        let mut hlen = 0usize;
        let status = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n";
        header[..status.len()].copy_from_slice(status);
        hlen += status.len();
        hlen += write_content_length(file_len, &mut header[hlen..]);
        header[hlen..hlen + 2].copy_from_slice(b"\r\n");
        hlen += 2;

        tcp_send(stream_cap, &header[..hlen]);
        tcp_send(stream_cap, &file_buf[..file_len]);

        // Yield to let smoltcp flush the TX buffer before sending FIN.
        // Without this, close_cap() may trigger a FIN before the data
        // segment is polled out and delivered to the host.
        for _ in 0..500 { sys_yield(); }
        close_cap(stream_cap);
    }
}
