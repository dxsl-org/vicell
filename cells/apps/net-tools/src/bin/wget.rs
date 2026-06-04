//! wget — HTTP/1.0 file downloader for ViOS.
//!
//! Usage: wget http://IP[:PORT][/path] <vfs_path>
//!
//! Downloads the URL body and writes it to `<vfs_path>` via VFS OP_WRITE IPC.
//! Mirrors curl.rs for the network side; replaces stdout printing with a VFS
//! write so the result is available to subsequent shell commands.

#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::println;
use ostd::syscall::{sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};

const NET_ENDPOINT: usize = 6;
const VFS_ENDPOINT: usize = 3;

const SOCKET_TCP:   u8 = 0x10;
const CONNECT:      u8 = 0x12;
const SEND_OP:      u8 = 0x13;
const RECV_OP:      u8 = 0x14;
const CLOSE:        u8 = 0x15;
const SOCKET_STATE: u8 = 0x19;
const OP_WRITE:     u8 = 0x04;

const RESP_BUF: usize = 4096;

#[no_mangle]
pub fn main() {
    // Read spawn args early to avoid ARGV_STASH_KEY race.
    let mut arg_buf = [0u8; 192];
    let arg_len = sys_spawn_args(&mut arg_buf);
    if arg_len == 0 {
        println("Usage: wget http://IP[:PORT][/path] <vfs_path>");
        return;
    }
    let args_str = match core::str::from_utf8(&arg_buf[..arg_len]) {
        Ok(s) => s.trim(),
        Err(_) => { println("wget: bad args"); return; }
    };
    let mut parts = args_str.splitn(2, ' ');
    let url = match parts.next() { Some(u) => u, None => { println("wget: missing URL"); return; } };
    let vfs_path = match parts.next() { Some(p) => p.trim(), None => { println("wget: missing output path"); return; } };

    let (host, port, path) = match parse_url(url) {
        Some(t) => t,
        None => { println("wget: invalid URL — expected http://IP[:PORT][/path]"); return; }
    };
    let addr = match resolve_host(host) {
        Some(a) => a,
        None => { println("wget: invalid host"); return; }
    };

    // SOCKET_TCP → cap_id.
    let socket_msg = [SOCKET_TCP, 0, 0, 0, 0, 0, 0, 0, 0];
    sys_send(NET_ENDPOINT, &socket_msg);
    let mut cap_reply = [0u8; 8];
    let cap_id = match sys_recv(0, &mut cap_reply) {
        SyscallResult::Ok(_) => u64::from_le_bytes(cap_reply),
        _ => { println("wget: SOCKET_TCP failed"); return; }
    };
    if cap_id == 0 { println("wget: no socket cap"); return; }

    // CONNECT.
    let mut conn_msg = [0u8; 15];
    conn_msg[0] = CONNECT;
    conn_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    conn_msg[9..13].copy_from_slice(&addr);
    conn_msg[13..15].copy_from_slice(&port.to_le_bytes());
    sys_send(NET_ENDPOINT, &conn_msg);
    let mut ack = [0u8; 1];
    match sys_recv(0, &mut ack) {
        SyscallResult::Ok(_) if ack[0] == 0x00 => {}
        _ => { println("wget: connect failed"); close_socket(cap_id); return; }
    }

    // Build and send GET request.
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

    let mut sent_bytes = 0usize;
    for _ in 0..500 {
        if sent_bytes >= request_len { break; }
        let rem_offset = 9 + sent_bytes;
        let rem_len = send_len - rem_offset;
        let mut retry = [0u8; 400];
        retry[0] = SEND_OP;
        retry[1..9].copy_from_slice(&cap_id.to_le_bytes());
        retry[9..9 + rem_len].copy_from_slice(&send_msg[rem_offset..send_len]);
        sys_send(NET_ENDPOINT, &retry[..9 + rem_len]);
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

    // Accumulate HTTP response.
    let mut response = [0u8; RESP_BUF];
    let mut resp_len: usize = 0;
    let mut recv_msg = [0u8; 13];
    recv_msg[0] = RECV_OP;
    recv_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    recv_msg[9..13].copy_from_slice(&512u32.to_le_bytes());

    'recv: for _ in 0..500 {
        let mut buf = [0u8; 512];
        sys_send(NET_ENDPOINT, &recv_msg);
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(_) => {
                let n = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                if n > 0 {
                    let copy = n.min(RESP_BUF - resp_len);
                    response[resp_len..resp_len + copy].copy_from_slice(&buf[..copy]);
                    resp_len += copy;
                    if resp_len >= RESP_BUF { break 'recv; }
                } else {
                    let st = query_state(cap_id);
                    if st == 0x06 || st == 0x00 {
                        let mut fb = [0u8; 512];
                        sys_send(NET_ENDPOINT, &recv_msg);
                        if let SyscallResult::Ok(_) = sys_recv(0, &mut fb) {
                            let fn_ = fb.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                            let copy = fn_.min(RESP_BUF - resp_len);
                            response[resp_len..resp_len + copy].copy_from_slice(&fb[..copy]);
                            resp_len += copy;
                        }
                        break 'recv;
                    }
                    sys_yield();
                }
            }
            _ => break,
        }
    }
    close_socket(cap_id);

    // Extract body (everything after the \r\n\r\n header separator).
    let resp = &response[..resp_len];
    let body = resp.windows(4).position(|w| w == b"\r\n\r\n")
        .map(|i| &resp[i + 4..])
        .unwrap_or(resp);

    if body.is_empty() {
        println("wget: empty response body");
        return;
    }

    // Write body to VFS via OP_WRITE IPC.
    let pb = vfs_path.as_bytes();
    let pl = pb.len().min(253);
    let cl = body.len().min(480usize.saturating_sub(pl));
    let mut req = [0u8; 4 + 253 + 480];
    req[0] = OP_WRITE;
    req[1] = pl as u8;
    req[2..4].copy_from_slice(&(cl as u16).to_le_bytes());
    req[4..4 + pl].copy_from_slice(&pb[..pl]);
    req[4 + pl..4 + pl + cl].copy_from_slice(&body[..cl]);
    sys_send(VFS_ENDPOINT, &req[..4 + pl + cl]);
    let mut r = [0u8; 1];
    match sys_recv(0, &mut r) {
        SyscallResult::Ok(_) if r[0] == 0 => {
            ostd::io::print("wget: saved ");
            ostd::io::print_usize(cl);
            ostd::io::print(" bytes to ");
            println(vfs_path);
        }
        _ => println("wget: VFS write failed"),
    }
}

fn query_state(cap_id: u64) -> u8 {
    let mut msg = [0u8; 9]; msg[0] = SOCKET_STATE;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut st = [0u8; 1];
    match sys_recv(0, &mut st) { SyscallResult::Ok(_) => st[0], _ => 0 }
}
fn close_socket(cap_id: u64) {
    let mut msg = [0u8; 9]; msg[0] = CLOSE;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut r = [0u8; 1]; let _ = sys_recv(0, &mut r);
}
fn wb(buf: &mut [u8], pos: usize, src: &[u8]) -> usize {
    buf[pos..pos + src.len()].copy_from_slice(src); pos + src.len()
}
fn parse_url(s: &str) -> Option<(&str, u16, &str)> {
    let rest = s.strip_prefix("http://")?;
    if rest.is_empty() { return None; }
    let (hp, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]), None => (rest, "/"),
    };
    let (host, port) = match hp.rfind(':') {
        Some(i) => (&hp[..i], parse_u16(&hp[i + 1..])?), None => (hp, 80u16),
    };
    if host.is_empty() { return None; }
    Some((host, port, path))
}
fn resolve_host(s: &str) -> Option<[u8; 4]> {
    match s {
        "gateway" | "host" => Some([10, 0, 2, 2]),
        "dns" => Some([10, 0, 2, 3]),
        "localhost" => Some([127, 0, 0, 1]),
        _ => parse_ipv4(s),
    }
}
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut it = s.splitn(5, '.');
    let a = po(it.next()?)?; let b = po(it.next()?)?;
    let c = po(it.next()?)?; let d = po(it.next()?)?;
    if it.next().is_some() { return None; }
    Some([a, b, c, d])
}
fn po(s: &str) -> Option<u8> {
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
