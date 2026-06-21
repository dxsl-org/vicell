#![no_std]
#![no_main]
extern crate ostd;

use ostd::io::println;
use ostd::syscall::sys_lookup_service;
use ostd::tls::{tls_close, tls_connect, tls_read, tls_write};

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, LookupService];

// example.com — 93.184.216.34
const EXAMPLE_IP: [u8; 4] = [93, 184, 216, 34];
const HTTPS_PORT: u16 = 443;
const HOSTNAME: &str = "example.com";

const GET_REQUEST: &[u8] = b"GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n";

#[no_mangle]
pub fn main() {
    println("[https-demo] TLS 1.3 HTTPS demo starting");

    let net_tid = match sys_lookup_service(api::syscall::service::NET) {
        Some(tid) => tid,
        None => {
            println("[https-demo] ERROR: net service not found");
            return;
        }
    };

    println("[https-demo] Connecting to example.com:443 via TLS 1.3...");
    let cap = tls_connect(net_tid, EXAMPLE_IP, HTTPS_PORT, HOSTNAME);
    if cap == 0 {
        println("[https-demo] ERROR: TLS handshake failed");
        return;
    }
    println("[https-demo] TLS handshake OK");

    // Send GET request in chunks (tls_write caps at 503 bytes; request fits in one).
    let mut sent = 0;
    while sent < GET_REQUEST.len() {
        let n = tls_write(net_tid, cap, &GET_REQUEST[sent..]);
        if n == 0 {
            println("[https-demo] ERROR: send stalled");
            tls_close(net_tid, cap);
            return;
        }
        sent += n;
    }

    // Read response and print the status line.
    let mut buf = [0u8; 512];
    let mut total = 0usize;
    let mut status_printed = false;

    for _ in 0..200 {
        let n = tls_read(net_tid, cap, &mut buf[total..]);
        if n == 0 {
            // No data yet — yield and retry.
            ostd::task::yield_now();
            continue;
        }
        total += n;

        // Once we have the status line, print it and stop.
        if !status_printed {
            if let Some(cr) = buf[..total].iter().position(|&b| b == b'\r') {
                if let Ok(status) = core::str::from_utf8(&buf[..cr]) {
                    println(status);
                    status_printed = true;
                }
            }
        }

        // Stop after receiving the header block.
        if total >= 4 {
            let w = &buf[..total];
            if w.windows(4).any(|s| s == b"\r\n\r\n") {
                break;
            }
        }
        if total >= buf.len() {
            break;
        }
    }

    if !status_printed && total > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..total.min(80)]) {
            println(s);
        }
    }

    tls_close(net_tid, cap);
    println("[https-demo] done");
}
