#![no_std]
#![no_main]
#![forbid(unsafe_code)]
extern crate alloc;
extern crate ostd;

use alloc::vec::Vec;
use ostd::app::{AppContext, AppEvent};
use ostd::clients::TcpStream;
use ostd::http::{HttpClient, TlsStream};
use ostd::io::println;
use ostd::json;
use ostd::syscall::sys_exit;

// `network = false` is correct: this cell never makes a direct network syscall.
// All net access is via IPC (Send/Recv) to the net service, which holds the
// network capability. The cap lives on the service, not on this client cell.
ostd::app_entry!(block_io = false, network = false, spawn = false, handler = smoke_handler);

// Host-side mock LLM, reachable from the QEMU guest via SLIRP (host = 10.0.2.2).
// Run `python tools/hypha-mock-llm/mock_proxy.py --plain` (port 8080) for HTTP
// and `python tools/hypha-mock-llm/mock_proxy.py` (port 8443) for TLS.
const MOCK_IP: [u8; 4] = [10, 0, 2, 2];
const HTTP_PORT: u16 = 8080;
const HTTPS_PORT: u16 = 8443;
const HOSTNAME: &str = "mock";

const PATH: &str = "/v1/chat/completions";
const CONTENT_TYPE: &str = "application/json";
// The mock echoes choices[0].message.content back from the prompt, so a >503-B
// body also exercises the TlsStream write-all loop (plan success criterion).
const PROMPT: &[u8] = br#"{"model":"mock","messages":[{"role":"user","content":"ping-roundtrip"}]}"#;

/// Drive one full request/response over `client`, drain the body, parse JSON,
/// and return the extracted `choices[0].message.content` string.
fn run_exchange<T>(mut client: HttpClient<T>) -> Result<alloc::string::String, ()>
where
    T: ostd::embedded_io::Read + ostd::embedded_io::Write,
{
    let (headers, mut body) = client.post(HOSTNAME, PATH, CONTENT_TYPE, PROMPT).map_err(|_| ())?;
    if headers.status != 200 {
        println("[http-smoke] non-200 status");
        return Err(());
    }

    // Drain the whole body (Content-Length or chunked) into a Vec.
    //
    // The body decoder never returns NeedMoreData (that variant is header-only):
    // a transport 0-read mid-body means the connection closed (truncation) and
    // surfaces as `Err(UnexpectedEof)`. A stalled TlsStream surfaces as `Err`
    // after exhausting its retry budget. BOTH are hard failures here — breaking
    // with an error, never spinning. `Ok(0)` is the only clean-completion exit.
    let mut transport = client.into_inner();
    let mut full: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match body.read(&mut transport, &mut chunk) {
            Ok(0) => break, // body complete per framing
            Ok(n) => full.extend_from_slice(&chunk[..n]),
            Err(ostd::http::HttpError::UnexpectedEof) => {
                println("[http-smoke] FAIL: connection closed mid-body (truncated)");
                return Err(());
            }
            Err(_) => return Err(()),
        }
    }

    // Parse JSON and extract the echoed content.
    let value: json::Value = json::from_slice(&full).map_err(|_| ())?;
    let content = json::get_str(&value["choices"][0], &["message", "content"]).ok_or(())?;
    Ok(alloc::string::String::from(content))
}

fn smoke_handler(_ctx: &mut AppContext, event: AppEvent) {
    match event {
        AppEvent::Init => run_smoke(),
        AppEvent::Shutdown | AppEvent::ShutdownWith { .. } => sys_exit(0),
        _ => {}
    }
}

fn run_smoke() {
    // TLS handshake in QEMU (ECDHE key exchange + SLIRP TCP) takes 20-50 s.
    // Reset the watchdog to 60 s so the cell isn't killed mid-handshake.
    // Heartbeat is in the base syscall set — no extra cap needed.
    ostd::syscall::sys_heartbeat(6000);

    println("[http-smoke] HTTP/JSON e2e smoke starting");

    // ── HTTP over TcpStream ────────────────────────────────────────────────
    println("[http-smoke] HTTP: connecting to 10.0.2.2:8080 ...");
    match TcpStream::connect(MOCK_IP, HTTP_PORT) {
        Ok(tcp) => match run_exchange(HttpClient::new(tcp)) {
            Ok(content) => {
                println("[http-smoke] HTTP content extracted:");
                println(&content);
                if content.is_empty() {
                    println("[http-smoke] HTTP FAIL: empty content");
                } else {
                    println("[http-smoke] HTTP PASS");
                }
            }
            Err(()) => println("[http-smoke] HTTP FAIL: exchange/parse error"),
        },
        Err(_) => println("[http-smoke] HTTP FAIL: connect (is the mock running on :8080 --plain?)"),
    }

    // ── HTTPS over TlsStream ───────────────────────────────────────────────
    println("[http-smoke] HTTPS: connecting to 10.0.2.2:8443 via TLS 1.3 ...");
    match TlsStream::connect(MOCK_IP, HTTPS_PORT, HOSTNAME) {
        Ok(tls) => match run_exchange(HttpClient::new(tls)) {
            Ok(content) => {
                println("[http-smoke] HTTPS content extracted:");
                println(&content);
                if content.is_empty() {
                    println("[http-smoke] HTTPS FAIL: empty content");
                } else {
                    println("[http-smoke] HTTPS PASS");
                }
            }
            Err(()) => println("[http-smoke] HTTPS FAIL: exchange/parse error"),
        },
        Err(_) => println("[http-smoke] HTTPS FAIL: connect (is the mock running on :8443?)"),
    }

    println("[http-smoke] done");
    sys_exit(0);
}
