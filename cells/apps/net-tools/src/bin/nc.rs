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

/// IPC opcodes for the net service (mirrors poll_driver::cell_opcodes).
const SOCKET_TCP: u8 = 0x10;
const CONNECT:    u8 = 0x12;
const SEND_OP:    u8 = 0x13;
const RECV_OP:    u8 = 0x14;
const CLOSE:      u8 = 0x15;
const LISTEN_OP:  u8 = 0x17;
const ACCEPT_OP:  u8 = 0x18;
const STATE_OP:   u8 = 0x19;

/// Payload sent and expected back from the echo server.
const HELLO: &[u8] = b"HELLO_VIOS\n";

/// nc <host_ip> <port>  |  nc -l <port>
///
/// Arguments are read from the state-stash argv slot published by the shell
/// via sys_set_spawn_args before spawning.
#[no_mangle]
pub fn main() {
    // ── Parse argv ───────────────────────────────────────────────────────────
    let mut arg_buf = [0u8; 64];
    let arg_len = sys_spawn_args(&mut arg_buf);
    if arg_len == 0 {
        println("Usage: nc <host> <port>  |  nc -l <port>");
        return;
    }
    let args_str = match core::str::from_utf8(&arg_buf[..arg_len]) {
        Ok(s) => s,
        Err(_) => { println("nc: bad args"); return; }
    };
    let mut parts = args_str.split_whitespace();
    let first = match parts.next() {
        Some(t) => t,
        None => { println("Usage: nc <host> <port>  |  nc -l <port>"); return; }
    };

    if first == "-l" {
        // Server mode: nc -l <port>
        let port = match parts.next().and_then(parse_u16) {
            Some(p) => p,
            None => { println("Usage: nc -l <port>"); return; }
        };
        server_mode(port);
        return;
    }

    // Client mode: first token is the host.
    let host = first;
    let port_str = match parts.next() {
        Some(p) => p,
        None => { println("Usage: nc <host> <port>"); return; }
    };
    let addr = match resolve_host(host) {
        Some(a) => a,
        None => { println("nc: invalid host"); return; }
    };
    let port: u16 = match parse_u16(port_str) {
        Some(p) => p,
        None => { println("nc: invalid port"); return; }
    };

    // ── SOCKET_TCP → cap_id ──────────────────────────────────────────────────
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
    // Track sent_bytes so each retry forwards only the unsent suffix, preventing
    // prefix duplication if smoltcp accepts a partial write (n < HELLO.len()).
    let mut sent_bytes = 0usize;
    for _ in 0..500 {
        if sent_bytes >= HELLO.len() { break; }
        let rem = &HELLO[sent_bytes..];
        let mut send_msg = [0u8; 9 + 11];
        send_msg[0] = SEND_OP;
        send_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
        send_msg[9..9 + rem.len()].copy_from_slice(rem);
        sys_send(NET_ENDPOINT, &send_msg[..9 + rem.len()]);
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

    // ── RECV echo — poll until data arrives ───────────────────────────────────
    let mut recv_msg = [0u8; 13];
    recv_msg[0] = RECV_OP;
    recv_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
    recv_msg[9..13].copy_from_slice(&256u32.to_le_bytes());

    for _ in 0..500 {
        let mut data = [0u8; 256];
        sys_send(NET_ENDPOINT, &recv_msg);
        match sys_recv(0, &mut data) {
            SyscallResult::Ok(_) if data[0] != 0 => {
                let end = data.iter().position(|&b| b == 0).unwrap_or(256);
                if let Ok(s) = core::str::from_utf8(&data[..end]) {
                    print(s);
                }
                break;
            }
            _ => { sys_yield(); }
        }
    }

    close_socket(cap_id);
}

/// nc -l <port> — listen, accept one connection, echo bytes to serial and
/// back to the peer, then close when the peer closes.
fn server_mode(port: u16) {
    // SOCKET_TCP → cap
    let socket_msg = [SOCKET_TCP, 0, 0, 0, 0, 0, 0, 0, 0];
    sys_send(NET_ENDPOINT, &socket_msg);
    let mut cap_reply = [0u8; 8];
    let cap = match sys_recv(0, &mut cap_reply) {
        SyscallResult::Ok(_) => u64::from_le_bytes(cap_reply),
        _ => { println("nc: SOCKET_TCP failed"); return; }
    };
    if cap == 0 { println("nc: no socket cap"); return; }

    // LISTEN [0x17][cap:8][port:2 LE] → [0x00] ok
    let mut listen_msg = [0u8; 11];
    listen_msg[0] = LISTEN_OP;
    listen_msg[1..9].copy_from_slice(&cap.to_le_bytes());
    listen_msg[9..11].copy_from_slice(&port.to_le_bytes());
    sys_send(NET_ENDPOINT, &listen_msg);
    let mut ack = [0u8; 1];
    match sys_recv(0, &mut ack) {
        SyscallResult::Ok(_) if ack[0] == 0x00 => {}
        _ => { println("nc: listen failed"); close_socket(cap); return; }
    }
    print("listening on ");
    ostd::io::print_usize(port as usize);
    println("");

    // ACCEPT [0x18][cap:8] → stream_cap, or u64::MAX = not ready yet.
    // Loop indefinitely — nc -l naturally blocks until a connection arrives.
    // The test harness enforces its own deadline via wait_for("connected", N).
    let mut accept_msg = [0u8; 9];
    accept_msg[0] = ACCEPT_OP;
    accept_msg[1..9].copy_from_slice(&cap.to_le_bytes());
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
    println("connected");

    // RECV loop: print to serial AND echo back. Exit when peer closes.
    let mut recv_msg = [0u8; 13];
    recv_msg[0] = RECV_OP;
    recv_msg[1..9].copy_from_slice(&stream_cap.to_le_bytes());
    recv_msg[9..13].copy_from_slice(&256u32.to_le_bytes());

    'recv: for _ in 0..500_000 {
        let mut data = [0u8; 256];
        sys_send(NET_ENDPOINT, &recv_msg);
        match sys_recv(0, &mut data) {
            SyscallResult::Ok(_) if data[0] != 0 => {
                let end = data.iter().position(|&b| b == 0).unwrap_or(256);
                if let Ok(s) = core::str::from_utf8(&data[..end]) {
                    print(s);
                }
                // Echo back to peer.
                let mut send_msg = [0u8; 9 + 256];
                send_msg[0] = SEND_OP;
                send_msg[1..9].copy_from_slice(&stream_cap.to_le_bytes());
                send_msg[9..9 + end].copy_from_slice(&data[..end]);
                sys_send(NET_ENDPOINT, &send_msg[..9 + end]);
                let mut cnt = [0u8; 4];
                let _ = sys_recv(0, &mut cnt);
            }
            SyscallResult::Ok(_) => {
                let st = query_state(stream_cap);
                if st == 0x06 || st == 0x00 { break 'recv; }
                sys_yield();
            }
            _ => break,
        }
    }
    close_socket(stream_cap);

    // Loop back: accept the next connection on the same listener.
    // Update accept_msg for the (unchanged) listen cap.
    accept_msg[1..9].copy_from_slice(&cap.to_le_bytes());

    // Re-enter the accept loop — use goto-style tail call by recursing
    // into the outer accept state. Rather than recursion, update stream_cap
    // inline by falling through to the top of the accept poll at the caller.
    // Simplest approach: loop the whole accept+recv block.
    loop {
        println("waiting for next connection");
        let next_cap: u64 = loop {
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
        println("connected");

        let mut rmsg = [0u8; 13];
        rmsg[0] = RECV_OP;
        rmsg[1..9].copy_from_slice(&next_cap.to_le_bytes());
        rmsg[9..13].copy_from_slice(&256u32.to_le_bytes());

        'r2: for _ in 0..500_000 {
            let mut data = [0u8; 256];
            sys_send(NET_ENDPOINT, &rmsg);
            match sys_recv(0, &mut data) {
                SyscallResult::Ok(_) if data[0] != 0 => {
                    let end = data.iter().position(|&b| b == 0).unwrap_or(256);
                    if let Ok(s) = core::str::from_utf8(&data[..end]) { print(s); }
                    let mut smsg = [0u8; 9 + 256];
                    smsg[0] = SEND_OP;
                    smsg[1..9].copy_from_slice(&next_cap.to_le_bytes());
                    smsg[9..9 + end].copy_from_slice(&data[..end]);
                    sys_send(NET_ENDPOINT, &smsg[..9 + end]);
                    let mut cnt = [0u8; 4]; let _ = sys_recv(0, &mut cnt);
                }
                SyscallResult::Ok(_) => {
                    let st = query_state(next_cap);
                    if st == 0x06 || st == 0x00 { break 'r2; }
                    sys_yield();
                }
                _ => break,
            }
        }
        close_socket(next_cap);
    }
}

/// Query SOCKET_STATE (0x19) → 1-byte smoltcp state code.
fn query_state(cap: u64) -> u8 {
    let mut msg = [0u8; 9];
    msg[0] = STATE_OP;
    msg[1..9].copy_from_slice(&cap.to_le_bytes());
    sys_send(NET_ENDPOINT, &msg);
    let mut st = [0u8; 1];
    match sys_recv(0, &mut st) {
        SyscallResult::Ok(_) => st[0],
        _ => 0x00,
    }
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

/// Resolve a hostname to an IPv4 address, falling back to literal parsing.
///
/// Static table only — no DNS. Aliases for the QEMU SLIRP environment.
fn resolve_host(s: &str) -> Option<[u8; 4]> {
    match s {
        "gateway" | "host" => Some([10, 0, 2, 2]),
        "dns"              => Some([10, 0, 2, 3]),
        "localhost"        => Some([127, 0, 0, 1]),
        _                  => parse_ipv4(s),
    }
}

/// Parse "a.b.c.d" into `[a, b, c, d]`.
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
