#![no_std]
#![no_main]
extern crate alloc;
extern crate ostd;

use alloc::vec::Vec;
use ostd::io::{print, println};
use ostd::syscall::{sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};

/// Net service cell task ID — matches init's fourth sys_spawn_from_path call.
const NET_ENDPOINT: usize = 5;

/// IPC opcodes (mirrors cells/services/net/src/poll_driver.rs cell_opcodes).
const SOCKET_TCP:   u8 = 0x10;
const CONNECT:      u8 = 0x12;
const SEND_OP:      u8 = 0x13;
const RECV_OP:      u8 = 0x14;
const CLOSE:        u8 = 0x15;
const SOCKET_STATE: u8 = 0x19; // Phase B opcode — Phase 1

/// curl http://IP[:PORT][/path]
///
/// HTTP/1.0 GET client. Sends a minimal request, accumulates the response on
/// the heap, and prints the status line + body to serial.
///
/// Limitation: response byte-count detection uses zero-scan (`rposition` on a
/// zeroed buffer). This is reliable for ASCII HTTP (no embedded 0x00 bytes in
/// headers or typical text bodies). Binary responses would be truncated at the
/// first zero byte — a RECV2 opcode with explicit length is a future fix.
#[no_mangle]
pub fn main() {
    // ── Parse argv ───────────────────────────────────────────────────────────
    let mut arg_buf = [0u8; 128];
    let arg_len = sys_spawn_args(&mut arg_buf);
    if arg_len == 0 {
        println("Usage: curl http://IP[:PORT][/path]");
        return;
    }
    let url = match core::str::from_utf8(&arg_buf[..arg_len]) {
        Ok(s) => s.trim(),
        Err(_) => { println("curl: bad args"); return; }
    };

    // ── Parse URL ────────────────────────────────────────────────────────────
    let (host, port, path) = match parse_url(url) {
        Some(t) => t,
        None => {
            println("curl: invalid URL — expected http://IP[:PORT][/path]");
            return;
        }
    };
    let addr = match parse_ipv4(host) {
        Some(a) => a,
        None => { println("curl: invalid IPv4 host"); return; }
    };

    // ── Guard: HTTP request must fit in the IPC payload budget (~391 bytes) ──
    // Request = "GET <path> HTTP/1.0\r\nHost: <host>\r\nConnection: close\r\n\r\n"
    // Fixed overhead = 4+11+6+2+19+2 = 44 bytes; add 9 for IPC header.
    let overhead = b"GET ".len() + b" HTTP/1.0\r\n".len()
        + b"Host: ".len() + b"\r\nConnection: close\r\n\r\n".len();
    if 9 + overhead + path.len() + host.len() > 400 {
        println("curl: URL too long");
        return;
    }

    // ── SOCKET_TCP → cap_id ──────────────────────────────────────────────────
    let socket_msg = [SOCKET_TCP, 0, 0, 0, 0, 0, 0, 0, 0];
    sys_send(NET_ENDPOINT, &socket_msg);
    let mut cap_reply = [0u8; 8];
    let cap_id = match sys_recv(0, &mut cap_reply) {
        SyscallResult::Ok(_) => u64::from_le_bytes(cap_reply),
        _ => { println("curl: SOCKET_TCP failed"); return; }
    };
    if cap_id == 0 {
        println("curl: no socket cap");
        return;
    }

    // ── CONNECT [0x12][cap:8][addr:4][port:2 LE] ─────────────────────────────
    let mut conn_msg = [0u8; 15];
    conn_msg[0] = CONNECT;
    conn_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    conn_msg[9..13].copy_from_slice(&addr);
    conn_msg[13..15].copy_from_slice(&port.to_le_bytes());
    sys_send(NET_ENDPOINT, &conn_msg);
    let mut ack = [0u8; 1];
    match sys_recv(0, &mut ack) {
        SyscallResult::Ok(_) if ack[0] == 0x00 => {}
        _ => {
            println("curl: connect failed");
            close_socket(cap_id);
            return;
        }
    }

    // ── Build SEND message: [SEND_OP][cap:8][GET headers] ────────────────────
    let mut send_msg = [0u8; 400];
    send_msg[0] = SEND_OP;
    send_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    let mut pos = 9usize;
    pos = wb(&mut send_msg, pos, b"GET ");
    pos = wb(&mut send_msg, pos, path.as_bytes());
    pos = wb(&mut send_msg, pos, b" HTTP/1.0\r\nHost: ");
    pos = wb(&mut send_msg, pos, host.as_bytes());
    pos = wb(&mut send_msg, pos, b"\r\nConnection: close\r\n\r\n");
    let send_len = pos;
    let request_len = send_len - 9;

    // ── Retry SEND until smoltcp confirms bytes buffered (TCP must reach ──────
    // Established first — SEND replies 0 while the handshake is in flight).
    // SEND reply is a 4-byte LE count (set by the net cell); sys_recv fills the
    // buffer, so u32::from_le_bytes(cnt) gives the real written count.
    for _ in 0..500 {
        sys_send(NET_ENDPOINT, &send_msg[..send_len]);
        let mut cnt = [0u8; 4];
        match sys_recv(0, &mut cnt) {
            SyscallResult::Ok(_) => {
                let n = u32::from_le_bytes(cnt) as usize;
                if n >= request_len { break; }
                if n == 0 { sys_yield(); }
            }
            _ => break,
        }
    }

    // ── Accumulate HTTP response ─────────────────────────────────────────────
    // sys_recv returns sender_id (not byte count). Zero the buffer each
    // iteration and use rposition scan to find how many bytes were written.
    let mut response: Vec<u8> = Vec::new();

    let mut recv_msg = [0u8; 13];
    recv_msg[0] = RECV_OP;
    recv_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    recv_msg[9..13].copy_from_slice(&2048u32.to_le_bytes());

    for _ in 0..500 {
        let mut buf = [0u8; 2048]; // zeroed each iteration — zero-scan relies on this
        sys_send(NET_ENDPOINT, &recv_msg);
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(_) => {
                let n = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                if n > 0 {
                    response.extend_from_slice(&buf[..n]);
                } else {
                    // 0-byte RECV: query TCP state to distinguish "no data yet"
                    // (Established) from "server closed" (CloseWait/Closed).
                    let st = query_state(cap_id);
                    if st == 0x06 || st == 0x00 {
                        // CloseWait or Closed: drain once more — data can be
                        // buffered in smoltcp alongside the arriving FIN.
                        let mut final_buf = [0u8; 2048];
                        sys_send(NET_ENDPOINT, &recv_msg);
                        if let SyscallResult::Ok(_) = sys_recv(0, &mut final_buf) {
                            let fn_ = final_buf.iter()
                                .rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                            if fn_ > 0 { response.extend_from_slice(&final_buf[..fn_]); }
                        }
                        break;
                    }
                    sys_yield();
                }
            }
            _ => break,
        }
    }

    // ── Print status line + body ─────────────────────────────────────────────
    if let Some(header_end) = response.windows(4).position(|w| w == b"\r\n\r\n") {
        // Status line = first line up to first \r.
        let status_end = response.iter().position(|&b| b == b'\r').unwrap_or(header_end);
        if let Ok(status) = core::str::from_utf8(&response[..status_end]) {
            println(status);
        }
        // Body = everything after the blank line.
        let body = &response[header_end + 4..];
        if let Ok(s) = core::str::from_utf8(body) {
            print(s);
        }
    } else if !response.is_empty() {
        if let Ok(s) = core::str::from_utf8(&response) {
            print(s);
        }
    } else {
        println("curl: empty response");
    }

    close_socket(cap_id);
}

/// Query SOCKET_STATE (0x19) and return the 1-byte smoltcp state code.
fn query_state(cap_id: u64) -> u8 {
    let mut msg = [0u8; 9];
    msg[0] = SOCKET_STATE;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut st = [0u8; 1];
    match sys_recv(0, &mut st) {
        SyscallResult::Ok(_) => st[0],
        _ => 0x00, // treat error as Closed
    }
}

/// Send CLOSE and drain the 1-byte acknowledgement.
fn close_socket(cap_id: u64) {
    let mut msg = [0u8; 9];
    msg[0] = CLOSE;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut r = [0u8; 1];
    let _ = sys_recv(0, &mut r);
}

/// Write `src` into `buf` at `pos`; returns new pos.
///
/// Caller must guarantee `pos + src.len() <= buf.len()` (enforced by the
/// URL-length guard above before any call to `wb`).
fn wb(buf: &mut [u8], pos: usize, src: &[u8]) -> usize {
    buf[pos..pos + src.len()].copy_from_slice(src);
    pos + src.len()
}

/// Parse `http://IP[:PORT][/path]` → `(host, port, path)`.
fn parse_url(s: &str) -> Option<(&str, u16, &str)> {
    let rest = s.strip_prefix("http://")?;
    if rest.is_empty() { return None; }
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None    => (rest, "/"),
    };
    let (host, port) = match host_port.rfind(':') {
        Some(i) => (&host_port[..i], parse_u16(&host_port[i + 1..])?),
        None    => (host_port, 80u16),
    };
    if host.is_empty() { return None; }
    Some((host, port, path))
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut it = s.splitn(5, '.');
    let a = parse_octet(it.next()?)?;
    let b = parse_octet(it.next()?)?;
    let c = parse_octet(it.next()?)?;
    let d = parse_octet(it.next()?)?;
    if it.next().is_some() { return None; }
    Some([a, b, c, d])
}

fn parse_octet(s: &str) -> Option<u8> {
    let mut n: u16 = 0;
    if s.is_empty() { return None; }
    for ch in s.bytes() {
        if !(b'0'..=b'9').contains(&ch) { return None; }
        n = n * 10 + (ch - b'0') as u16;
        if n > 255 { return None; }
    }
    Some(n as u8)
}

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
