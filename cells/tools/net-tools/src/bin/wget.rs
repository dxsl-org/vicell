//! wget — HTTP/1.0 file downloader for ViCell.
//!
//! Usage: wget http://IP[:PORT][/path] <vfs_path>
//!
//! Downloads the URL body and writes it to `<vfs_path>` via typed VFS Write IPC.

#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::println;
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};
use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;

const RESP_BUF: usize = 4096;

api::declare_syscalls![Send, Recv, Log, StateRestore, LookupService];

#[no_mangle]
pub fn main() {
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

    // ── Resolve service endpoints ─────────────────────────────────────────────
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => { println("wget: no net service"); return; }
    };
    let vfs_ep = match sys_lookup_service(service::VFS) {
        Some(ep) => ep,
        None => { println("wget: no vfs service"); return; }
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
            _ => { println("wget: connect failed"); return; }
        },
        _ => { println("wget: TcpConnect syscall failed"); return; }
    };

    // ── Build and send HTTP GET request ───────────────────────────────────────
    let overhead = b"GET ".len() + b" HTTP/1.0\r\nHost: ".len()
        + b"\r\nConnection: close\r\n\r\n".len();
    if overhead + path.len() + host.len() > 500 {
        println("wget: URL too long");
        close_socket(cap_id, net_ep);
        return;
    }
    let mut request_data = [0u8; 512];
    let mut pos = 0usize;
    pos = wb(&mut request_data, pos, b"GET ");
    pos = wb(&mut request_data, pos, path.as_bytes());
    pos = wb(&mut request_data, pos, b" HTTP/1.0\r\nHost: ");
    pos = wb(&mut request_data, pos, host.as_bytes());
    pos = wb(&mut request_data, pos, b"\r\nConnection: close\r\n\r\n");
    let request_len = pos;

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
                        let st = query_state(cap_id, net_ep);
                        if st == 0x06 || st == 0x00 {
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
    close_socket(cap_id, net_ep);

    // ── Extract body and write to VFS ─────────────────────────────────────────
    let resp = &response[..resp_len];
    let body = resp.windows(4).position(|w| w == b"\r\n\r\n")
        .map(|i| &resp[i + 4..])
        .unwrap_or(resp);

    if body.is_empty() {
        println("wget: empty response body");
        return;
    }

    // Typed postcard IPC — VFS dropped the raw OP_* byte protocol in Phase 27;
    // the old OP_WRITE frame decoded as garbage and every write failed.
    // Content cap mirrors the 512-byte IPC frame minus path + envelope.
    let cl = body.len().min(440usize.saturating_sub(vfs_path.len()));
    let mut vfs_req = [0u8; 512];
    let req = api::ipc::VfsRequest::Write { path: vfs_path, content: &body[..cl] };
    let n = match api::ipc::encode(&req, &mut vfs_req) {
        Ok(s) => s.len(),
        Err(_) => { println("wget: request too large"); return; }
    };
    sys_send(vfs_ep, &vfs_req[..n]);
    let mut r = [0u8; 64];
    match sys_recv(0, &mut r) {
        SyscallResult::Ok(_) => match api::ipc::decode::<api::ipc::VfsResponse>(&r) {
            Ok(api::ipc::VfsResponse::Ok) => {
                ostd::io::print("wget: saved ");
                ostd::io::print_usize(cl);
                ostd::io::print(" bytes to ");
                println(vfs_path);
            }
            _ => println("wget: VFS write failed"),
        },
        _ => println("wget: VFS write failed"),
    }
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
            _ => 0,
        },
        _ => 0,
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

fn wb(buf: &mut [u8], pos: usize, src: &[u8]) -> usize {
    buf[pos..pos + src.len()].copy_from_slice(src);
    pos + src.len()
}

fn parse_url(s: &str) -> Option<(&str, u16, &str)> {
    let rest = s.strip_prefix("http://")?;
    if rest.is_empty() { return None; }
    let (hp, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None    => (rest, "/"),
    };
    let (host, port) = match hp.rfind(':') {
        Some(i) => (&hp[..i], parse_u16(&hp[i + 1..])?),
        None    => (hp, 80u16),
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
    let a = po(it.next()?)?;
    let b = po(it.next()?)?;
    let c = po(it.next()?)?;
    let d = po(it.next()?)?;
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
