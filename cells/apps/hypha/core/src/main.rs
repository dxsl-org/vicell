//! Hypha `core` — the agent brain (P1: interactive chat).
//!
//! Spawns the `llm-gateway` cell, then loops: read a line from stdin (UART),
//! keep the conversation in heap, ask the gateway over IPC, print the reply.
//! `core` talks to the gateway by tid (the spawn return value) — no service
//! registry in P1. It needs the `spawn` capability; nothing else dangerous
//! (no network, no block-I/O) — side-effecting tools arrive as separate
//! capability-gated Cells in later phases.

#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

use agent_proto::{LlmReply, LlmRequest};
use alloc::string::String;
use alloc::vec::Vec;
use ostd::io::{print, println, stdin};
use ostd::syscall::{sys_exit, sys_recv, sys_send, sys_spawn_from_path, SyscallResult};

api::declare_manifest!(block_io = false, network = false, spawn = true);
api::declare_syscalls![Send, Recv, Read, Log, SpawnFromPath];

const GATEWAY_PATH: &str = "/bin/llm-gateway";
// Prompt must fit one IPC message (os-gap G5: Grant for large prompts later).
const PROMPT_MAX: usize = 3500;

#[no_mangle]
pub fn main() {
    println("Hypha — ViCell's first AI agent (P1 chat). Type 'exit' to quit.");

    let gw = match sys_spawn_from_path(GATEWAY_PATH) {
        SyscallResult::Ok(tid) => tid,
        _ => {
            println("[hypha] ERROR: cannot spawn llm-gateway");
            return;
        }
    };

    let mut conversation: Vec<(&'static str, String)> = Vec::new();
    let sin = stdin();

    loop {
        print("\nyou> ");
        let mut line = String::new();
        if sin.read_line(&mut line).is_err() {
            break;
        }
        let user = line.trim();
        if user.is_empty() {
            continue;
        }
        if user == "exit" || user == "quit" {
            break;
        }

        conversation.push(("user", String::from(user)));
        let prompt = render_prompt(&conversation);

        match ask(gw, &prompt) {
            Ok(reply) => {
                print("hypha> ");
                println(reply.as_str());
                conversation.push(("assistant", reply));
            }
            Err(e) => {
                print("[hypha] error: ");
                println(e.as_str());
                // Drop the failed turn so it doesn't poison later context.
                conversation.pop();
            }
        }
    }

    println("[hypha] bye");
    // Cells must sys_exit, not return from main: a plain `#[no_mangle] main`
    // that returns falls through to address 0 → instruction page fault
    // (scause=0xc, sepc=0x0). Exit cleanly so the supervisor reaps us.
    sys_exit(0);
}

/// Flatten the heap conversation into a single role-tagged transcript. P1 sends
/// this as one prompt string; the gateway wraps it as the user turn. Trimmed
/// from the front if it would exceed the one-message IPC budget (os-gap G5).
fn render_prompt(conv: &[(&'static str, String)]) -> String {
    let mut s = String::new();
    for (role, text) in conv {
        s.push_str(role);
        s.push_str(": ");
        s.push_str(text);
        s.push('\n');
    }
    s.push_str("assistant: ");
    trim_front(s, PROMPT_MAX)
}

/// One request/response round-trip with the gateway over IPC.
fn ask(gw: usize, prompt: &str) -> Result<String, String> {
    let req = LlmRequest::Complete { prompt };
    let mut out = [0u8; 4096];
    let encoded = postcard::to_slice(&req, &mut out)
        .map_err(|_| String::from("prompt too large for one IPC message"))?;

    match sys_send(gw, encoded) {
        SyscallResult::Ok(_) => {}
        _ => return Err(String::from("send to gateway failed")),
    }

    let mut buf = [0u8; 4096];
    match sys_recv(0, &mut buf) {
        SyscallResult::Ok(_sender) => match postcard::from_bytes::<LlmReply>(&buf) {
            Ok(LlmReply::Text(t)) => Ok(t),
            Ok(LlmReply::Error(e)) => Err(e),
            Err(_) => Err(String::from("bad LlmReply encoding")),
        },
        _ => Err(String::from("no reply from gateway")),
    }
}

/// Keep the tail of `s` within `max` bytes (drop oldest turns), char-boundary safe.
fn trim_front(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut start = s.len() - max;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    String::from(&s[start..])
}
