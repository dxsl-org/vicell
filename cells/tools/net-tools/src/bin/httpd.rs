//! httpd — minimal HTTP/1.0 file server for ViCell.
//!
//! Usage: httpd <port> <vfs_path>
//!
//! Listens for TCP connections on <port>.  For each connection, reads the
//! HTTP request (discards it), reads <vfs_path> **fresh from VFS on every
//! request** (no caching), and responds with HTTP/1.0 200 OK + current file
//! content.  Returns HTTP 404 when the file is absent or empty.  Loops
//! forever, serving one connection at a time.

#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::{print, println};
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};
use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;

api::declare_syscalls![Send, Recv, Log, StateRestore, LookupService];

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

fn close_cap(cap: u32, net_ep: usize) {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpClose { cap_id: cap }, &mut req_buf)
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &req_buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv(0, &mut r);
}

fn query_state(cap: u32, net_ep: usize) -> u8 {
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::SocketState { cap_id: cap }, &mut req_buf)
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

/// Read a VFS file via typed `ReadAsync` + `Poll` IPC (≤480 bytes per reply).
///
/// The raw OP_READ byte protocol this used to speak was dropped when VFS went
/// typed-postcard (Phase 27) — every read then returned garbage. ReadAsync
/// works for both RamFS and disk-backed (/data) paths.
fn vfs_read(path: &str, buf: &mut [u8], vfs_ep: usize) -> usize {
    use api::ipc::{VfsRequest, VfsResponse};
    let mut ipc = [0u8; 512];

    let n = match api::ipc::encode(&VfsRequest::ReadAsync { path }, &mut ipc) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    sys_send(vfs_ep, &ipc[..n]);
    let handle = match sys_recv(0, &mut ipc) {
        SyscallResult::Ok(_) => match api::ipc::decode::<VfsResponse>(&ipc) {
            Ok(VfsResponse::PendingHandle(h)) => h,
            _ => return 0,
        },
        _ => return 0,
    };

    let n = match api::ipc::encode(&VfsRequest::Poll { handle }, &mut ipc) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    sys_send(vfs_ep, &ipc[..n]);
    match sys_recv(0, &mut ipc) {
        SyscallResult::Ok(_) => match api::ipc::decode::<VfsResponse>(&ipc) {
            Ok(VfsResponse::Data(data)) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                len
            }
            _ => 0,
        },
        _ => 0,
    }
}

/// Send bytes to a TCP socket cap via TcpSend, retrying until all bytes are buffered.
fn tcp_send(cap: u32, data: &[u8], net_ep: usize) {
    let mut sent = 0usize;
    for _ in 0..1000 {
        if sent >= data.len() { break; }
        let rem = &data[sent..];
        let chunk = rem.len().min(480);
        let mut send_buf = [0u8; IPC_BUF_SIZE];
        let send_len = api::ipc::encode(
            &NetRequest::TcpSend { cap_id: cap, data: &rem[..chunk] },
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

/// Drain the HTTP request until the header terminator `\r\n\r\n` is seen.
fn drain_request(cap: u32, net_ep: usize) {
    let mut recv_req_buf = [0u8; IPC_BUF_SIZE];
    let recv_req_len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id: cap, buf_len: 256 },
        &mut recv_req_buf,
    ).map(|b| b.len()).unwrap_or(0);

    for _ in 0..200 {
        sys_send(net_ep, &recv_req_buf[..recv_req_len]);
        let mut data_buf = [0u8; IPC_BUF_SIZE];
        match sys_recv(0, &mut data_buf) {
            SyscallResult::Ok(_) => {
                match api::ipc::decode::<NetResponse>(&data_buf) {
                    Ok(NetResponse::Data(b)) if !b.is_empty() => {
                        if b.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                    }
                    Ok(NetResponse::Data(_)) => {
                        let st = query_state(cap, net_ep);
                        if st == 0x06 || st == 0x00 { break; }
                        sys_yield();
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
}

/// Write "Content-Length: N\r\n" as ASCII into `out`.  Returns byte count.
fn write_content_length(n: usize, out: &mut [u8]) -> usize {
    let prefix = b"Content-Length: ";
    let mut pos = 0usize;
    out[pos..pos + prefix.len()].copy_from_slice(prefix);
    pos += prefix.len();
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

    // ── Resolve service endpoints ─────────────────────────────────────────────
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => { println("httpd: no net service"); return; }
    };
    let vfs_ep = match sys_lookup_service(service::VFS) {
        Some(ep) => ep,
        None => { println("httpd: no vfs service"); return; }
    };

    // ── TcpListen (atomic create + listen) ───────────────────────────────────
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
            _ => { println("httpd: listen failed"); return; }
        },
        _ => { println("httpd: TcpListen syscall failed"); return; }
    };
    print("httpd: listening on ");
    ostd::io::print_usize(port as usize);
    println("");

    // Pre-encode TcpAccept for the listen cap (reused across iterations).
    let mut accept_req_buf = [0u8; IPC_BUF_SIZE];
    let accept_req_len = api::ipc::encode(
        &NetRequest::TcpAccept { cap_id: listen_cap },
        &mut accept_req_buf,
    ).map(|b| b.len()).unwrap_or(0);

    // Accept loop — serve one connection at a time.
    loop {
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

        drain_request(stream_cap, net_ep);

        let mut file_buf = [0u8; 4096];
        let file_len = vfs_read(path, &mut file_buf, vfs_ep);

        if file_len == 0 {
            let not_found = b"HTTP/1.0 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n";
            tcp_send(stream_cap, not_found, net_ep);
        } else {
            let mut header = [0u8; 128];
            let mut hlen = 0usize;
            let status = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n";
            header[..status.len()].copy_from_slice(status);
            hlen += status.len();
            hlen += write_content_length(file_len, &mut header[hlen..]);
            header[hlen..hlen + 2].copy_from_slice(b"\r\n");
            hlen += 2;
            tcp_send(stream_cap, &header[..hlen], net_ep);
            tcp_send(stream_cap, &file_buf[..file_len], net_ep);
        }

        // Yield to let smoltcp flush TX before sending FIN.
        for _ in 0..500 { sys_yield(); }
        close_cap(stream_cap, net_ep);
    }
}
