#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::{print, println};
use ostd::syscall::{sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};

/// Net service cell task ID.
///
/// The kernel spawns `init` (ID 1) and a `user_hello` smoke-test task (ID 2)
/// before the init binary runs. Init then spawns: vfs=3, config=4, input=5,
/// net=6, compositor=7, shell=8. Verified from QEMU serial log.
const NET_ENDPOINT: usize = 6;

/// IPC opcodes (mirrors cells/services/net/src/poll_driver.rs cell_opcodes).
const SOCKET_TCP:   u8 = 0x10;
const CONNECT:      u8 = 0x12;
const SEND_OP:      u8 = 0x13;
const RECV_OP:      u8 = 0x14;
const CLOSE:        u8 = 0x15;
const SOCKET_STATE: u8 = 0x19; // Phase B opcode

/// Maximum accumulated response size (stack-allocated, no heap required).
///
/// The SAS (Single Address Space) design means all cells share one VA region.
/// Using `extern crate alloc` (ostd's 4 MB static heap) causes the BSS segment
/// to overlap with other cells' already-mapped pages, producing instruction page
/// faults. A fixed stack buffer avoids that 4 MB BSS entirely.
///
/// HTTP/1.0 test response is <50 bytes; 4096 bytes is ample headroom.
const RESP_BUF: usize = 4096;

/// curl http://IP[:PORT][/path]
///
/// HTTP/1.0 GET client. Accumulates up to RESP_BUF bytes of response on the
/// stack and prints the status line + body.
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

    // ── Guard: HTTP request must fit in the IPC payload budget ───────────────
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

    // ── Retry SEND until all request bytes buffered ───────────────────────────
    // Track sent_bytes; each retry forwards only the unsent suffix so a partial
    // write (n < request_len) doesn't duplicate the already-buffered prefix.
    let mut sent_bytes = 0usize;
    for _ in 0..500 {
        if sent_bytes >= request_len { break; }
        let rem_offset = 9 + sent_bytes;
        let rem_len = send_len - rem_offset;
        // Rebuild: [SEND_OP][cap:8][remaining request bytes]
        let mut retry_msg = [0u8; 400];
        retry_msg[0] = SEND_OP;
        retry_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
        retry_msg[9..9 + rem_len].copy_from_slice(&send_msg[rem_offset..send_len]);
        sys_send(NET_ENDPOINT, &retry_msg[..9 + rem_len]);
        let mut cnt = [0u8; 4];
        match sys_recv(0, &mut cnt) {
            SyscallResult::Ok(_) => {
                let n = u32::from_le_bytes(cnt) as usize;
                sent_bytes += n;
                if n == 0 { sys_yield(); }
            }
            _ => break,
        }
    }

    // ── Accumulate HTTP response into fixed stack buffer ──────────────────────
    // sys_recv returns sender_id (not byte count). Zero buffer each iteration
    // and use rposition scan for data boundary (zero-scan, ASCII-safe).
    //
    // Fixed buffer avoids the 4 MB alloc BSS that conflicts with other cells'
    // VA mappings in the shared SAS page table.
    let mut response = [0u8; RESP_BUF];
    let mut resp_len: usize = 0;

    let mut recv_msg = [0u8; 13];
    recv_msg[0] = RECV_OP;
    recv_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    recv_msg[9..13].copy_from_slice(&512u32.to_le_bytes()); // 512B chunks

    for _ in 0..500 {
        let mut buf = [0u8; 512]; // zeroed each iteration — zero-scan relies on this
        sys_send(NET_ENDPOINT, &recv_msg);
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(_) => {
                let n = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                if n > 0 {
                    let copy = n.min(RESP_BUF - resp_len);
                    response[resp_len..resp_len + copy].copy_from_slice(&buf[..copy]);
                    resp_len += copy;
                    if resp_len >= RESP_BUF { break; }
                } else {
                    // 0-byte RECV: check state to distinguish "no data yet" vs FIN.
                    let st = query_state(cap_id);
                    if st == 0x06 || st == 0x00 {
                        // CloseWait/Closed: drain one final RECV before exiting.
                        let mut final_buf = [0u8; 512];
                        sys_send(NET_ENDPOINT, &recv_msg);
                        if let SyscallResult::Ok(_) = sys_recv(0, &mut final_buf) {
                            let fn_ = final_buf.iter()
                                .rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                            let copy = fn_.min(RESP_BUF - resp_len);
                            response[resp_len..resp_len + copy].copy_from_slice(&final_buf[..copy]);
                            resp_len += copy;
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
    let resp = &response[..resp_len];
    if let Some(header_end) = resp.windows(4).position(|w| w == b"\r\n\r\n") {
        let status_end = resp.iter().position(|&b| b == b'\r').unwrap_or(header_end);
        if let Ok(status) = core::str::from_utf8(&resp[..status_end]) {
            println(status);
        }
        let body = &resp[header_end + 4..];
        if let Ok(s) = core::str::from_utf8(body) {
            print(s);
        }
    } else if resp_len > 0 {
        if let Ok(s) = core::str::from_utf8(resp) {
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
        _ => 0x00,
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
fn wb(buf: &mut [u8], pos: usize, src: &[u8]) -> usize {
    buf[pos..pos + src.len()].copy_from_slice(src);
    pos + src.len()
}

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
