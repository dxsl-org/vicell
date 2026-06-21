//! MQTT 3.1.1 QoS-0 client cell.
//!
//! Usage:
//!   mqtt publish  <host:port> <topic> <payload>
//!   mqtt subscribe <host:port> <topic>
//!
//! Only QoS 0 (fire-and-forget publish, at-most-once subscribe).
#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::{print, println};
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};
use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;

api::declare_syscalls![Send, Recv, Log, StateRestore, LookupService];

#[no_mangle]
pub fn main() {
    let mut arg_buf = [0u8; 128];
    let arg_len = sys_spawn_args(&mut arg_buf);
    if arg_len == 0 {
        println("Usage: mqtt publish  host:port topic payload");
        println("       mqtt subscribe host:port topic");
        return;
    }
    let args_str = match core::str::from_utf8(&arg_buf[..arg_len]) {
        Ok(s) => s,
        Err(_) => { println("mqtt: bad args"); return; }
    };
    let mut parts = args_str.split_whitespace();
    let subcmd   = match parts.next() { Some(s) => s, None => { println("mqtt: missing subcommand"); return; } };
    let hostport = match parts.next() { Some(s) => s, None => { println("mqtt: missing host:port"); return; } };
    let topic    = match parts.next() { Some(s) => s, None => { println("mqtt: missing topic"); return; } };

    let (host, port) = match hostport.rfind(':') {
        Some(i) => (&hostport[..i], parse_u16(&hostport[i + 1..]).unwrap_or(1883)),
        None    => (hostport, 1883u16),
    };
    let addr = match resolve_host(host) {
        Some(a) => a,
        None => { println("mqtt: invalid host"); return; }
    };

    // ── Resolve net service endpoint ──────────────────────────────────────────
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => { println("mqtt: no net service"); return; }
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
            _ => { println("mqtt: tcp connect failed"); return; }
        },
        _ => { println("mqtt: TcpConnect syscall failed"); return; }
    };
    println("connected");

    if !mqtt_handshake(cap_id, net_ep) {
        println("mqtt: CONNACK rejected");
        close_socket(cap_id, net_ep);
        return;
    }

    match subcmd {
        "publish"   => {
            let payload = parts.next().unwrap_or("");
            do_publish(cap_id, topic, payload, net_ep);
        }
        "subscribe" => { do_subscribe(cap_id, topic, net_ep); }
        _ => { println("mqtt: unknown subcommand"); }
    }
    close_socket(cap_id, net_ep);
}

/// Send MQTT CONNECT and verify CONNACK `[0x20 0x02 0x00 0x00]`.
fn mqtt_handshake(cap: u32, net_ep: usize) -> bool {
    tcp_send(cap, &[
        0x10, 0x10,
        0x00, 0x04, b'M', b'Q', b'T', b'T',
        0x04,
        0x02,
        0x00, 0x3C,
        0x00, 0x04, b'v', b'i', b'o', b's',
    ], net_ep);
    let mut buf = [0u8; 256];
    let n = mqtt_recv(cap, &mut buf, 500, net_ep);
    n >= 4 && buf[0] == 0x20 && buf[3] == 0x00
}

/// Build and send a PUBLISH packet (QoS 0).
fn do_publish(cap: u32, topic: &str, payload: &str, net_ep: usize) {
    let tb = topic.as_bytes();
    let pb = payload.as_bytes();
    if tb.len() > 64  { println("mqtt: topic too long (max 64 bytes)"); return; }
    if pb.len() > 256 { println("mqtt: payload too long (max 256 bytes)"); return; }
    let remaining = 2 + tb.len() + pb.len();
    let mut pkt = [0u8; 340];
    let mut rl  = [0u8; 4];
    let rl_len  = encode_remaining_len(remaining, &mut rl);
    pkt[0] = 0x30;
    pkt[1..1 + rl_len].copy_from_slice(&rl[..rl_len]);
    let mut p = 1 + rl_len;
    pkt[p]     = (tb.len() >> 8) as u8;
    pkt[p + 1] = tb.len() as u8;
    p += 2;
    pkt[p..p + tb.len()].copy_from_slice(tb); p += tb.len();
    pkt[p..p + pb.len()].copy_from_slice(pb); p += pb.len();
    tcp_send(cap, &pkt[..p], net_ep);
    println("published");
}

/// Send SUBSCRIBE, verify SUBACK, then poll and print incoming PUBLISH payloads.
fn do_subscribe(cap: u32, topic: &str, net_ep: usize) {
    let tb = topic.as_bytes();
    let remaining = 5 + tb.len();
    let mut pkt = [0u8; 96];
    let mut rl  = [0u8; 4];
    let rl_len  = encode_remaining_len(remaining, &mut rl);
    pkt[0] = 0x82;
    pkt[1..1 + rl_len].copy_from_slice(&rl[..rl_len]);
    let mut p = 1 + rl_len;
    pkt[p] = 0x00; pkt[p + 1] = 0x01; p += 2;
    pkt[p] = (tb.len() >> 8) as u8; pkt[p + 1] = tb.len() as u8; p += 2;
    pkt[p..p + tb.len()].copy_from_slice(tb); p += tb.len();
    pkt[p] = 0x00; p += 1;
    tcp_send(cap, &pkt[..p], net_ep);

    let mut buf = [0u8; 256];
    let n = mqtt_recv(cap, &mut buf, 5000, net_ep);
    if n == 0 || buf[0] != 0x90 { println("mqtt: SUBACK not received"); return; }
    println("subscribed");

    for _ in 0..10_000usize {
        let mut data = [0u8; 256];
        let n = mqtt_recv_once(cap, &mut data, net_ep);
        if n < 4 || data[0] != 0x30 { sys_yield(); continue; }
        let topic_len     = (data[2] as usize) << 8 | data[3] as usize;
        let payload_start = 4 + topic_len;
        let payload_end   = (2 + data[1] as usize).min(n);
        if payload_end <= payload_start { continue; }
        if let Ok(s) = core::str::from_utf8(&data[payload_start..payload_end]) {
            print(s);
            if !s.ends_with('\n') { println(""); }
        }
    }
}

/// Send bytes over TCP via TcpSend; retries until all sent.
fn tcp_send(cap: u32, data: &[u8], net_ep: usize) {
    let mut sent = 0usize;
    for _ in 0..500 {
        if sent >= data.len() { break; }
        let chunk = (data.len() - sent).min(256);
        let mut send_buf = [0u8; IPC_BUF_SIZE];
        let send_len = api::ipc::encode(
            &NetRequest::TcpSend { cap_id: cap, data: &data[sent..sent + chunk] },
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
                        sent += n;
                        if n == 0 { sys_yield(); }
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
}

/// Send ONE TcpRecv; return bytes copied into `buf` (0 = nothing available yet).
fn mqtt_recv_once(cap: u32, buf: &mut [u8; 256], net_ep: usize) -> usize {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id: cap, buf_len: 256 },
        &mut req_buf,
    ).map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    match sys_recv(0, &mut resp_buf) {
        SyscallResult::Ok(_) => {
            match api::ipc::decode::<NetResponse>(&resp_buf) {
                Ok(NetResponse::Data(b)) if !b.is_empty() => {
                    // MQTT remaining-length is data[1]; clamp to actual bytes received.
                    let total = if b.len() >= 2 {
                        (2 + b[1] as usize).min(b.len()).min(256)
                    } else {
                        b.len().min(256)
                    };
                    buf[..total].copy_from_slice(&b[..total]);
                    total
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Poll until an MQTT packet arrives; yield between each poll.
fn mqtt_recv(cap: u32, buf: &mut [u8; 256], max_polls: usize, net_ep: usize) -> usize {
    for _ in 0..max_polls {
        let n = mqtt_recv_once(cap, buf, net_ep);
        if n > 0 { return n; }
        sys_yield();
    }
    0
}

fn close_socket(cap: u32, net_ep: usize) {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpClose { cap_id: cap }, &mut req_buf)
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv(0, &mut r);
}

fn encode_remaining_len(mut n: usize, out: &mut [u8; 4]) -> usize {
    let mut i = 0;
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 { b |= 0x80; }
        out[i] = b;
        i += 1;
        if n == 0 || i == 4 { break; }
    }
    i
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
