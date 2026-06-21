#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::{print, println};
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};
use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;

/// Payload sent and expected back from the echo server.
const HELLO: &[u8] = b"HELLO_ViCell\n";

api::declare_syscalls![Send, Recv, Log, StateRestore, LookupService];

/// nc <host_ip> <port>  |  nc -l <port>
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

    // ── Resolve net service endpoint ──────────────────────────────────────────
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => { println("nc: no net service"); return; }
    };

    if first == "-l" {
        let port = match parts.next().and_then(parse_u16) {
            Some(p) => p,
            None => { println("Usage: nc -l <port>"); return; }
        };
        server_mode(port, net_ep);
        return;
    }

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

    // ── TcpConnect (atomic create + connect) ─────────────────────────────────
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(
        &NetRequest::TcpConnect { addr, port },
        &mut req_buf,
    ).map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let cap_id = match sys_recv(0, &mut resp_buf) {
        SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&resp_buf) {
            Ok(NetResponse::CapId(c)) => c,
            _ => { println("nc: connect failed"); return; }
        },
        _ => { println("nc: TcpConnect syscall failed"); return; }
    };
    println("connected");

    // ── Send "HELLO_ViCell\n" via TcpSend with retry ──────────────────────────
    let mut sent_bytes = 0usize;
    for _ in 0..500 {
        if sent_bytes >= HELLO.len() { break; }
        let rem = &HELLO[sent_bytes..];
        let mut send_buf = [0u8; IPC_BUF_SIZE];
        let send_len = api::ipc::encode(
            &NetRequest::TcpSend { cap_id, data: rem },
            &mut send_buf,
        ).map(|b| b.len()).unwrap_or(0);
        sys_send(net_ep, &send_buf[..send_len]);
        let mut cnt_buf = [0u8; IPC_BUF_SIZE];
        match sys_recv(0, &mut cnt_buf) {
            SyscallResult::Ok(_) => {
                match api::ipc::decode::<NetResponse>(&cnt_buf) {
                    Ok(NetResponse::Data(b)) if b.len() >= 4 => {
                        let mut arr = [0u8; 4];
                        arr.copy_from_slice(&b[0..4]);
                        let n = u32::from_le_bytes(arr) as usize;
                        sent_bytes += n;
                        if n == 0 { sys_yield(); }
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }

    // ── Recv echo — poll until data arrives ───────────────────────────────────
    let mut recv_req_buf = [0u8; IPC_BUF_SIZE];
    let recv_req_len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id, buf_len: 256 },
        &mut recv_req_buf,
    ).map(|b| b.len()).unwrap_or(0);

    for _ in 0..500 {
        sys_send(net_ep, &recv_req_buf[..recv_req_len]);
        let mut data_buf = [0u8; IPC_BUF_SIZE];
        match sys_recv(0, &mut data_buf) {
            SyscallResult::Ok(_) => {
                match api::ipc::decode::<NetResponse>(&data_buf) {
                    Ok(NetResponse::Data(b)) if !b.is_empty() => {
                        if let Ok(s) = core::str::from_utf8(b) { print(s); }
                        break;
                    }
                    _ => { sys_yield(); }
                }
            }
            _ => break,
        }
    }

    close_socket(cap_id, net_ep);
}

/// nc -l <port> — listen, accept one connection, echo bytes to serial and
/// back to the peer, then close when the peer closes.
fn server_mode(port: u16, net_ep: usize) {
    // TcpListen (atomic create + listen)
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(
        &NetRequest::TcpListen { port },
        &mut req_buf,
    ).map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let listen_cap = match sys_recv(0, &mut resp_buf) {
        SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&resp_buf) {
            Ok(NetResponse::CapId(c)) => c,
            _ => { println("nc: listen failed"); return; }
        },
        _ => { println("nc: TcpListen syscall failed"); return; }
    };
    print("listening on ");
    ostd::io::print_usize(port as usize);
    println("");

    // Pre-encode TcpAccept for the listen cap (reused across accept polls).
    let mut accept_req_buf = [0u8; IPC_BUF_SIZE];
    let accept_req_len = api::ipc::encode(
        &NetRequest::TcpAccept { cap_id: listen_cap },
        &mut accept_req_buf,
    ).map(|b| b.len()).unwrap_or(0);

    let stream_cap: u32 = loop {
        sys_send(net_ep, &accept_req_buf[..accept_req_len]);
        let mut r = [0u8; IPC_BUF_SIZE];
        match sys_recv(0, &mut r) {
            SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&r) {
                Ok(NetResponse::CapId(c)) => break c,
                _ => { sys_yield(); }
            },
            _ => { sys_yield(); }
        }
    };
    println("connected");

    serve_connection(stream_cap, net_ep);

    // Accept loop — keep accepting connections on the same listener.
    loop {
        println("waiting for next connection");
        let next_cap: u32 = loop {
            sys_send(net_ep, &accept_req_buf[..accept_req_len]);
            let mut r = [0u8; IPC_BUF_SIZE];
            match sys_recv(0, &mut r) {
                SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&r) {
                    Ok(NetResponse::CapId(c)) => break c,
                    _ => { sys_yield(); }
                },
                _ => { sys_yield(); }
            }
        };
        println("connected");
        serve_connection(next_cap, net_ep);
    }
}

/// Recv loop: print received bytes to serial and echo back to peer.
/// Exits when the peer closes the connection.
fn serve_connection(cap: u32, net_ep: usize) {
    let mut recv_req_buf = [0u8; IPC_BUF_SIZE];
    let recv_req_len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id: cap, buf_len: 256 },
        &mut recv_req_buf,
    ).map(|b| b.len()).unwrap_or(0);

    'recv: for _ in 0..500_000 {
        sys_send(net_ep, &recv_req_buf[..recv_req_len]);
        let mut data_buf = [0u8; IPC_BUF_SIZE];
        match sys_recv(0, &mut data_buf) {
            SyscallResult::Ok(_) => {
                match api::ipc::decode::<NetResponse>(&data_buf) {
                    Ok(NetResponse::Data(b)) if !b.is_empty() => {
                        if let Ok(s) = core::str::from_utf8(b) { print(s); }
                        // Echo back to peer.
                        let mut echo_buf = [0u8; IPC_BUF_SIZE];
                        let echo_len = api::ipc::encode(
                            &NetRequest::TcpSend { cap_id: cap, data: b },
                            &mut echo_buf,
                        ).map(|e| e.len()).unwrap_or(0);
                        sys_send(net_ep, &echo_buf[..echo_len]);
                        let mut cnt_buf = [0u8; IPC_BUF_SIZE];
                        let _ = sys_recv(0, &mut cnt_buf);
                    }
                    Ok(NetResponse::Data(_)) => {
                        let st = query_state(cap, net_ep);
                        if st == 0x06 || st == 0x00 { break 'recv; }
                        sys_yield();
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
    close_socket(cap, net_ep);
}

fn query_state(cap_id: u32, net_ep: usize) -> u8 {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::SocketState { cap_id }, &mut req_buf)
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    match sys_recv(0, &mut resp_buf) {
        SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&resp_buf) {
            Ok(NetResponse::State(s)) => s,
            _ => 0x00,
        },
        _ => 0x00,
    }
}

fn close_socket(cap_id: u32, net_ep: usize) {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpClose { cap_id }, &mut req_buf)
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv(0, &mut resp_buf);
}

fn resolve_host(s: &str) -> Option<[u8; 4]> {
    match s {
        "gateway" | "host" => Some([10, 0, 2, 2]),
        "dns"              => Some([10, 0, 2, 3]),
        "localhost"        => Some([127, 0, 0, 1]),
        _                  => parse_ipv4(s),
    }
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
