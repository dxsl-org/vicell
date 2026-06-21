#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::{print, println};
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};
use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;

/// Maximum accumulated response size (stack-allocated; avoids the 4 MB alloc BSS).
const RESP_BUF: usize = 4096;

api::declare_syscalls![Send, Recv, Log, StateRestore, LookupService];

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
        None => { println("curl: invalid URL — expected http://IP[:PORT][/path]"); return; }
    };
    let addr = match resolve_host(host) {
        Some(a) => a,
        None => { println("curl: invalid host"); return; }
    };

    // ── Guard: HTTP request must fit in the request_data stack buffer ─────────
    let overhead = b"GET ".len() + b" HTTP/1.0\r\nHost: ".len()
        + b"\r\nConnection: close\r\n\r\n".len();
    if overhead + path.len() + host.len() > 500 {
        println("curl: URL too long");
        return;
    }

    // ── Resolve net service endpoint ──────────────────────────────────────────
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => { println("curl: no net service"); return; }
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
            _ => { println("curl: connect failed"); return; }
        },
        _ => { println("curl: TcpConnect syscall failed"); return; }
    };

    // ── Build HTTP GET request into a stack buffer ────────────────────────────
    let mut request_data = [0u8; 512];
    let mut pos = 0usize;
    pos = wb(&mut request_data, pos, b"GET ");
    pos = wb(&mut request_data, pos, path.as_bytes());
    pos = wb(&mut request_data, pos, b" HTTP/1.0\r\nHost: ");
    pos = wb(&mut request_data, pos, host.as_bytes());
    pos = wb(&mut request_data, pos, b"\r\nConnection: close\r\n\r\n");
    let request_len = pos;

    // ── Send HTTP request via TcpSend with retry ──────────────────────────────
    let mut sent_bytes = 0usize;
    for _ in 0..500 {
        if sent_bytes >= request_len { break; }
        let rem = &request_data[sent_bytes..request_len];
        let chunk = rem.len().min(480);
        let mut send_buf = [0u8; IPC_BUF_SIZE];
        let send_len = api::ipc::encode(
            &NetRequest::TcpSend { cap_id, data: &rem[..chunk] },
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

    // ── Accumulate HTTP response via TcpRecv ──────────────────────────────────
    let mut response = [0u8; RESP_BUF];
    let mut resp_len = 0usize;

    // Pre-encode the TcpRecv request once and reuse it across all iterations.
    let mut recv_req_buf = [0u8; IPC_BUF_SIZE];
    let recv_req_len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id, buf_len: 500 },
        &mut recv_req_buf,
    ).map(|b| b.len()).unwrap_or(0);

    'recv: for _ in 0..500 {
        sys_send(net_ep, &recv_req_buf[..recv_req_len]);
        let mut data_buf = [0u8; IPC_BUF_SIZE];
        match sys_recv(0, &mut data_buf) {
            SyscallResult::Ok(_) => {
                match api::ipc::decode::<NetResponse>(&data_buf) {
                    Ok(NetResponse::Data(b)) if !b.is_empty() => {
                        let n = b.len().min(RESP_BUF - resp_len);
                        response[resp_len..resp_len + n].copy_from_slice(&b[..n]);
                        resp_len += n;
                        if resp_len >= RESP_BUF { break 'recv; }
                    }
                    Ok(NetResponse::Data(_)) => {
                        // 0 bytes: check TCP state to distinguish "nothing yet" from FIN.
                        let st = query_state(cap_id, net_ep);
                        if st == 0x06 || st == 0x00 {
                            // CloseWait / Closed — drain one final recv before exiting.
                            sys_send(net_ep, &recv_req_buf[..recv_req_len]);
                            let mut fb = [0u8; IPC_BUF_SIZE];
                            if let SyscallResult::Ok(_) = sys_recv(0, &mut fb) {
                                if let Ok(NetResponse::Data(b)) = api::ipc::decode::<NetResponse>(&fb) {
                                    let n = b.len().min(RESP_BUF - resp_len);
                                    response[resp_len..resp_len + n].copy_from_slice(&b[..n]);
                                    resp_len += n;
                                }
                            }
                            break 'recv;
                        }
                        sys_yield();
                    }
                    _ => break,
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
        if let Ok(s) = core::str::from_utf8(body) { print(s); }
    } else if resp_len > 0 {
        if let Ok(s) = core::str::from_utf8(resp) { print(s); }
    } else {
        println("curl: empty response");
    }

    close_socket(cap_id, net_ep);
}

/// Send `NetRequest::SocketState` and return the 1-byte smoltcp state code.
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

/// Send `NetRequest::TcpClose` and drain the acknowledgement.
fn close_socket(cap_id: u32, net_ep: usize) {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpClose { cap_id }, &mut req_buf)
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv(0, &mut resp_buf);
}

/// Append `src` into `buf` at `pos`; returns new pos.
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
