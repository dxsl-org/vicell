//! HTTP round-trip to the proxy over either plain TCP or TLS.
//!
//! Plaintext (`--plain` mock, easier to test) uses `NetClient` directly and
//! detects connection close cleanly via `Err(ViError::Unknown)` (the net service
//! signals FIN/RST that way). TLS uses the `ostd::tls` helpers (net-service
//! mediated). Both stream the request out (chunked) and read until EOF / cap.

use alloc::string::String;
use alloc::vec::Vec;
use ostd::clients::NetClient;
use ostd::syscall::sys_lookup_service;
use ostd::tls::{tls_close, tls_connect, tls_read, tls_write};
use ostd::ViError;

const MAX_RESPONSE: usize = 64 * 1024;
const MAX_IDLE: u32 = 600;
const CHUNK: usize = 1024;

/// Send `request`, return the raw response bytes.
pub fn roundtrip(
    use_tls: bool,
    host: &str,
    ip: [u8; 4],
    port: u16,
    request: &[u8],
) -> Result<Vec<u8>, String> {
    if use_tls {
        tls(host, ip, port, request)
    } else {
        plain(ip, port, request)
    }
}

fn plain(ip: [u8; 4], port: u16, request: &[u8]) -> Result<Vec<u8>, String> {
    let mut nc = NetClient::new();
    let sock = nc
        .tcp_connect(ip, port)
        .map_err(|_| String::from("TCP connect failed"))?;
    ostd::io::println("[gw] TCP connected; sending");

    // tcp_send returns WouldBlock until the socket reaches Established (the
    // handshake completes a few net-service poll cycles after connect). Retry
    // each chunk with yields so the SYN-ACK has time to land.
    let mut sent = 0;
    while sent < request.len() {
        let end = (sent + CHUNK).min(request.len());
        let mut attempts = 0u32;
        loop {
            match nc.tcp_send(sock, &request[sent..end]) {
                Ok(()) => break,
                Err(_) => {
                    attempts += 1;
                    if attempts > 3000 {
                        let _ = nc.tcp_close(sock);
                        return Err(String::from("tcp_send failed (connect timeout?)"));
                    }
                    ostd::task::yield_now();
                }
            }
        }
        sent = end;
    }
    ostd::io::println("[gw] sent; reading");

    let mut resp = Vec::new();
    let mut idle = 0u32;
    loop {
        match nc.tcp_recv(sock, CHUNK as u32) {
            Ok(d) if !d.is_empty() => {
                idle = 0;
                resp.extend_from_slice(&d);
                if resp.len() > MAX_RESPONSE {
                    break;
                }
            }
            Ok(_) => {
                idle += 1;
                if idle > MAX_IDLE {
                    break;
                }
                ostd::task::yield_now();
            }
            Err(ViError::Unknown) => break, // EOF: net service signalled FIN/RST
            Err(_) => break,
        }
    }
    let _ = nc.tcp_close(sock);
    Ok(resp)
}

fn tls(host: &str, ip: [u8; 4], port: u16, request: &[u8]) -> Result<Vec<u8>, String> {
    let net = sys_lookup_service(api::syscall::service::NET)
        .ok_or_else(|| String::from("net service not found"))?;
    let cap = tls_connect(net, ip, port, host);
    if cap == 0 {
        return Err(String::from("TLS handshake failed"));
    }
    ostd::io::println("[gw] TLS ok; sending");

    let mut sent = 0;
    while sent < request.len() {
        let n = tls_write(net, cap, &request[sent..]);
        if n == 0 {
            tls_close(net, cap);
            return Err(String::from("send stalled"));
        }
        sent += n;
    }
    ostd::io::println("[gw] sent; reading");

    let mut resp = Vec::new();
    let mut chunk = [0u8; 512];
    let mut idle = 0u32;
    loop {
        let n = tls_read(net, cap, &mut chunk);
        if n == 0 {
            idle += 1;
            if idle > MAX_IDLE {
                break;
            }
            ostd::task::yield_now();
            continue;
        }
        idle = 0;
        resp.extend_from_slice(&chunk[..n]);
        if resp.len() > MAX_RESPONSE {
            break;
        }
    }
    tls_close(net, cap);
    Ok(resp)
}
