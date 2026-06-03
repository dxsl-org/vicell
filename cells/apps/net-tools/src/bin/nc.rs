#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::println;
use ostd::syscall::{sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};

/// Net service cell task ID — matches init's fourth sys_spawn_from_path call:
/// init=1, vfs=2, config=3, input=4, net=5.
const NET_ENDPOINT: usize = 5;

/// IPC opcodes for the net service (mirrors poll_driver::cell_opcodes).
const SOCKET_TCP: u8 = 0x10;
const CONNECT:    u8 = 0x12;
const SEND_OP:    u8 = 0x13;
const RECV_OP:    u8 = 0x14;
const CLOSE:      u8 = 0x15;

/// Payload sent and expected back from the echo server.
const HELLO: &[u8] = b"HELLO_VIOS\n";

/// nc <host_ip> <port> — connect, send HELLO_VIOS, print echo, close.
///
/// Arguments are read from the state-stash argv slot published by the shell
/// via sys_set_spawn_args before spawning (gives us "10.0.2.2 12345").
#[no_mangle]
pub fn main() {
    // ── Parse argv ───────────────────────────────────────────────────────────
    let mut arg_buf = [0u8; 64];
    let arg_len = sys_spawn_args(&mut arg_buf);
    if arg_len == 0 {
        println("Usage: nc <host> <port>");
        return;
    }
    let args_str = match core::str::from_utf8(&arg_buf[..arg_len]) {
        Ok(s) => s,
        Err(_) => { println("nc: bad args"); return; }
    };
    let mut parts = args_str.split_whitespace();
    let host = match parts.next() {
        Some(h) => h,
        None => { println("Usage: nc <host> <port>"); return; }
    };
    let port_str = match parts.next() {
        Some(p) => p,
        None => { println("Usage: nc <host> <port>"); return; }
    };
    let addr = match parse_ipv4(host) {
        Some(a) => a,
        None => { println("nc: invalid IPv4 address"); return; }
    };
    let port: u16 = match parse_u16(port_str) {
        Some(p) => p,
        None => { println("nc: invalid port"); return; }
    };

    // ── SOCKET_TCP → cap_id ──────────────────────────────────────────────────
    // Message: [opcode:1]  (no cap needed for SOCKET_TCP — net cell ignores it)
    let socket_msg = [SOCKET_TCP, 0, 0, 0, 0, 0, 0, 0, 0];
    sys_send(NET_ENDPOINT, &socket_msg);
    let mut cap_reply = [0u8; 8];
    let cap_id = match sys_recv(0, &mut cap_reply) {
        SyscallResult::Ok(_) => u64::from_le_bytes(cap_reply),
        _ => { println("nc: SOCKET_TCP failed"); return; }
    };
    if cap_id == 0 {
        println("nc: no socket cap returned");
        return;
    }

    // ── CONNECT [0x12][cap:8][addr:4][port:2] ────────────────────────────────
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
            println("nc: connect failed");
            close_socket(cap_id);
            return;
        }
    }

    println("connected");

    // ── SEND "HELLO_VIOS\n" — retry until all bytes buffered ─────────────────
    // The net cell returns a 4-byte LE byte-count. 0 = not established yet.
    // Build message: [0x13][cap:8][payload]
    let mut send_msg = [0u8; 9 + 11]; // opcode + cap + "HELLO_VIOS\n"
    send_msg[0] = SEND_OP;
    send_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    send_msg[9..9 + HELLO.len()].copy_from_slice(HELLO);

    // Retry until the net cell confirms bytes were buffered (n > 0 means
    // smoltcp reached Established and send_slice accepted the data).
    for _ in 0..500 {
        sys_send(NET_ENDPOINT, &send_msg);
        let mut cnt = [0u8; 4];
        match sys_recv(0, &mut cnt) {
            SyscallResult::Ok(_) => {
                let n = u32::from_le_bytes(cnt) as usize;
                if n >= HELLO.len() { break; }
                if n == 0 { sys_yield(); }
            }
            _ => break,
        }
    }

    // ── RECV echo — poll until data arrives ───────────────────────────────────
    // Message: [0x14][cap:8][buf_len:4 LE]
    // Reply: 0–n bytes. When 0 bytes arrive, kernel copies nothing → buffer stays 0.
    // "HELLO_VIOS\n" starts with 'H' (0x48), so buffer[0] != 0 means data arrived.
    let mut recv_msg = [0u8; 13];
    recv_msg[0] = RECV_OP;
    recv_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    let buf_len_le = 256u32.to_le_bytes();
    recv_msg[9..13].copy_from_slice(&buf_len_le);

    for _ in 0..500 {
        let mut data = [0u8; 256];
        sys_send(NET_ENDPOINT, &recv_msg);
        match sys_recv(0, &mut data) {
            SyscallResult::Ok(_) if data[0] != 0 => {
                // Find end of payload (first zero or full 256 bytes).
                let end = data.iter().position(|&b| b == 0).unwrap_or(256);
                if let Ok(s) = core::str::from_utf8(&data[..end]) {
                    ostd::io::print(s);
                }
                break;
            }
            _ => { sys_yield(); }
        }
    }

    close_socket(cap_id);
}

/// Send CLOSE [0x15][cap:8] and drain the 1-byte reply.
fn close_socket(cap_id: u64) {
    let mut msg = [0u8; 9];
    msg[0] = CLOSE;
    msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut r = [0u8; 1];
    let _ = sys_recv(0, &mut r);
}

/// Parse "a.b.c.d" into `[a, b, c, d]`.
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut it = s.splitn(5, '.');
    let a = parse_octet(it.next()?)?;
    let b = parse_octet(it.next()?)?;
    let c = parse_octet(it.next()?)?;
    let d = parse_octet(it.next()?)?;
    if it.next().is_some() { return None; } // more than 4 parts
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
