//! Hypha `llm-gateway` — the LLM network service cell.
//!
//! Receives [`LlmRequest`] over IPC, does an HTTP chat-completion round-trip to
//! an OpenAI-compatible endpoint, and replies [`LlmReply`]. Two transports
//! (see `transport.rs`):
//! - **plaintext** (`USE_TLS = false`, default): plain TCP via `NetClient` to a
//!   plain-HTTP mock (`tools/hypha-mock-llm/mock_proxy.py --plain`, port 8080).
//!   Easiest to test — no TLS variable.
//! - **TLS** (`USE_TLS = true`): TLS 1.3 via the net service (port 8443).
//!
//! Either way the cell holds **no** network capability — it only does IPC to the
//! `net` cell. `core` spawns this cell and talks to it by tid (no registry in P1).

#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

mod http;
mod transport;

use agent_proto::{LlmReply, LlmRequest};
use alloc::string::String;
use ostd::app::{AppContext, AppEvent};
use ostd::io::{print, print_usize, println};
use ostd::runtime::CellRuntime;

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, LookupService];

// ── Configuration ────────────────────────────────────────────────────────────
// Plaintext (default): the NetClient.tcp_send contract bug (os-gap G16) is fixed
// and the transport retries until the socket is Established. TLS is currently
// avoided — the net cell's embedded-tls handshake faults on a real server
// (os-gap G17, load page fault). Flip to true once G17 is resolved.
const USE_TLS: bool = false;
// 10.0.2.2 = QEMU user-net gateway = the host (pin the IP — os-gap G3).
const PROXY_IP: [u8; 4] = [10, 0, 2, 2];
const PROXY_PORT: u16 = if USE_TLS { 8443 } else { 8080 };
const PROXY_HOST: &str = "10.0.2.2";
const MODEL: &str = "claude-sonnet-4-6";
// Reply text must fit one IPC message (os-gap G5: Grant streaming comes later).
const IPC_REPLY_MAX: usize = 3800;

#[no_mangle]
pub fn main() {
    println("[hypha/llm-gateway] service ready");
    // no_heartbeat: an LLM round-trip can exceed the default 5 s watchdog.
    CellRuntime::new().no_heartbeat().run(|ctx, ev| match ev {
        AppEvent::Message { sender_tid, data } | AppEvent::RawMessage { sender_tid, data } => {
            handle(ctx, sender_tid, &data);
        }
        AppEvent::Shutdown | AppEvent::ShutdownWith { .. } => ostd::syscall::sys_exit(0),
        _ => {}
    });
}

/// Decode one request, run the completion, reply (truncated to one IPC message).
fn handle(ctx: &AppContext, sender: usize, data: &[u8]) {
    let reply = match postcard::from_bytes::<LlmRequest>(data) {
        Ok(LlmRequest::Complete { prompt }) => match complete(MODEL, prompt) {
            Ok(text) => classify_reply(text),
            Err(e) => LlmReply::Error(e),
        },
        Err(_) => LlmReply::Error(String::from("bad LlmRequest encoding")),
    };
    let mut buf = [0u8; 4096];
    if let Ok(bytes) = postcard::to_slice(&reply, &mut buf) {
        let _ = ctx.send(sender, bytes);
    }
}

/// One-shot chat completion. Non-streaming, single inline prompt (P1).
fn complete(model: &str, prompt: &str) -> Result<String, String> {
    let body = http::build_chat_body(model, prompt);
    let request = http::build_post(PROXY_HOST, "/v1/chat/completions", &body);

    print("[gw] ");
    println(if USE_TLS { "TLS mode -> :8443" } else { "plaintext mode -> :8080" });

    let resp = transport::roundtrip(USE_TLS, PROXY_HOST, PROXY_IP, PROXY_PORT, request.as_bytes())?;

    print("[gw] response bytes: ");
    print_usize(resp.len());
    println("");

    let body = http::http_body(&resp).ok_or_else(|| String::from("no HTTP body in response"))?;
    let content =
        http::extract_content(body).ok_or_else(|| String::from("no content field in response"))?;
    Ok(content)
}

/// Inspect the raw completion text: if it starts with `TOOL_CALL:` return
/// `ToolCalls`; otherwise return `Text` (truncated to the IPC budget).
fn classify_reply(text: String) -> LlmReply {
    if let Some(call) = http::extract_tool_call(&text) {
        LlmReply::ToolCalls(alloc::vec![call])
    } else {
        LlmReply::Text(fit(text, IPC_REPLY_MAX))
    }
}

/// Truncate `s` to at most `max` bytes on a char boundary, marking truncation.
fn fit(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str(" …[truncated: P1 4KB IPC cap — Grant streaming later]");
    s
}
