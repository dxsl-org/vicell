//! Hypha `tool-fs` — filesystem tool cell (P2).
//!
//! Receives [`AgentToolRequest::Invoke`] from `core`, dispatches to
//! [`ostd::clients::VfsClient`] (VFS service via IPC — no `block_io` cap needed),
//! and replies [`AgentToolResponse::Ok`] or [`AgentToolResponse::Err`].
//!
//! Supported tools:
//! - `read_file`  `{"path":"..."}` → `{"content":"..."}`
//! - `write_file` `{"path":"...","content":"..."}` → `{"ok":true}`
//! - `list_dir`   `{"path":"..."}` → `{"files":["a","b",...]}`

#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

use agent_proto::{AgentToolRequest, AgentToolResponse};
use alloc::string::String;
use ostd::app::{AppContext, AppEvent};
use ostd::clients::VfsClient;
use ostd::io::println;
use ostd::runtime::CellRuntime;
use ostd::syscall::sys_exit;

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, LookupService];

#[no_mangle]
pub fn main() {
    println("[tool-fs] ready");
    CellRuntime::new().no_heartbeat().run(|ctx, ev| match ev {
        AppEvent::Message { sender_tid, data } | AppEvent::RawMessage { sender_tid, data } => {
            handle(ctx, sender_tid, &data);
        }
        AppEvent::Shutdown | AppEvent::ShutdownWith { .. } => sys_exit(0),
        _ => {}
    });
}

fn handle(ctx: &AppContext, sender: usize, data: &[u8]) {
    let reply = match postcard::from_bytes::<AgentToolRequest<'_>>(data) {
        Ok(AgentToolRequest::Invoke { name, args_json }) => dispatch(name, args_json),
        Err(_) => AgentToolResponse::Err {
            message: String::from("bad AgentToolRequest encoding"),
        },
    };
    let mut buf = [0u8; 4096];
    if let Ok(bytes) = postcard::to_slice(&reply, &mut buf) {
        let _ = ctx.send(sender, bytes);
    }
}

fn dispatch(name: &str, args_json: &str) -> AgentToolResponse {
    match name {
        "read_file" => {
            let path = args_extract_str(args_json, "path").unwrap_or("/data/notes.txt");
            let mut vfs = VfsClient::new();
            match vfs.read_file(path) {
                Ok(bytes) => {
                    // Limit content returned to 2 KB — a tool result should be concise.
                    let text = core::str::from_utf8(&bytes).unwrap_or("[binary data]");
                    let truncated: String = text.chars().take(2048).collect();
                    AgentToolResponse::Ok {
                        result_json: json_obj_str("content", &truncated),
                    }
                }
                Err(e) => AgentToolResponse::Err {
                    message: alloc::format!("read_file {}: {:?}", path, e),
                },
            }
        }
        "write_file" => {
            let path = args_extract_str(args_json, "path").unwrap_or("/data/hypha-out.txt");
            let content = args_extract_str(args_json, "content").unwrap_or("");
            let mut vfs = VfsClient::new();
            match vfs.write_file(path, content.as_bytes()) {
                Ok(()) => AgentToolResponse::Ok {
                    result_json: String::from("{\"ok\":true}"),
                },
                Err(e) => AgentToolResponse::Err {
                    message: alloc::format!("write_file {}: {:?}", path, e),
                },
            }
        }
        "list_dir" => {
            let path = args_extract_str(args_json, "path").unwrap_or("/data");
            let mut vfs = VfsClient::new();
            match vfs.list_dir(path) {
                Ok(raw) => {
                    let text = core::str::from_utf8(&raw).unwrap_or("");
                    let entries_json: String = text
                        .split('\n')
                        .filter(|e| !e.is_empty())
                        .fold(String::new(), |mut acc, entry| {
                            if !acc.is_empty() {
                                acc.push(',');
                            }
                            acc.push('"');
                            acc.push_str(&json_escape(entry));
                            acc.push('"');
                            acc
                        });
                    AgentToolResponse::Ok {
                        result_json: alloc::format!("{{\"files\":[{}]}}", entries_json),
                    }
                }
                Err(e) => AgentToolResponse::Err {
                    message: alloc::format!("list_dir {}: {:?}", path, e),
                },
            }
        }
        other => AgentToolResponse::Err {
            message: alloc::format!("unknown tool: {}", other),
        },
    }
}

/// Extract a plain string field from a simple JSON args object.
/// Handles basic `\"` escaping; does not decode other escape sequences.
fn args_extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let search = alloc::format!("\"{}\"", key);
    let mut idx = json.find(search.as_str())? + search.len();
    let bytes = json.as_bytes();
    while idx < bytes.len() && matches!(bytes[idx], b' ' | b'\t' | b':') {
        idx += 1;
    }
    if idx >= bytes.len() || bytes[idx] != b'"' {
        return None;
    }
    idx += 1; // skip opening quote
    let start = idx;
    while idx < bytes.len() && bytes[idx] != b'"' {
        if bytes[idx] == b'\\' {
            idx += 1; // skip escaped char
        }
        idx += 1;
    }
    Some(&json[start..idx])
}

/// Wrap a single key/value pair into a JSON object string: `{"key":"value"}`.
fn json_obj_str(key: &str, value: &str) -> String {
    alloc::format!("{{\"{}\":\"{}\"}}", json_escape(key), json_escape(value))
}

/// Escape special characters for use inside a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Format control characters as \uXXXX
                let code = c as u32;
                out.push('\\');
                out.push('u');
                for shift in [12u32, 8, 4, 0] {
                    let nibble = (code >> shift) & 0xF;
                    out.push(char::from_digit(nibble, 16).unwrap_or('0'));
                }
            }
            c => out.push(c),
        }
    }
    out
}
