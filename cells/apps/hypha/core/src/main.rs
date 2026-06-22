//! Hypha `core` — the agent brain (P2: tool-augmented chat).
//!
//! Spawns `llm-gateway` + `tool-fs`, then loops: read a line from stdin (UART),
//! keep the conversation in heap, run an agentic turn (LLM + optional tool
//! sub-loop), print the final reply. `core` talks to both cells by tid (no
//! service registry — os-gap G13). It holds only the `spawn` capability; no
//! network, no block-I/O — side effects are delegated to capability-gated Cells.

#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

use agent_proto::{AgentToolRequest, AgentToolResponse, LlmReply, LlmRequest, ToolCall};
use alloc::string::String;
use alloc::vec::Vec;
use ostd::io::{print, println, stdin};
use ostd::syscall::{sys_exit, sys_recv, sys_send, sys_spawn_from_path, SyscallResult};

api::declare_manifest!(block_io = false, network = false, spawn = true);
api::declare_syscalls![Send, Recv, Read, Log, SpawnFromPath];

const GATEWAY_PATH: &str = "/bin/llm-gateway";
const TOOL_FS_PATH: &str = "/bin/tool-fs";
// Prompt must fit one IPC message (os-gap G5: Grant for large prompts later).
const PROMPT_MAX: usize = 3500;
// Tool call agentic loop guard — prevents infinite tool chains.
const MAX_TOOL_ROUNDS: usize = 5;
// System preamble prepended to every prompt so the LLM knows about tools.
// Kept short to preserve prompt budget.
const SYSTEM_PREAMBLE: &str = "\
system: You are Hypha, a helpful AI agent inside Cellos OS. \
You have file-system tools. When you need a tool, reply with ONLY this line (nothing else): \
TOOL_CALL: {\"name\":\"TOOL\",\"args\":{ARGS_JSON}} \
Available tools: read_file({\"path\":\"...\"}), write_file({\"path\":\"...\",\"content\":\"...\"}), list_dir({\"path\":\"...\"}). \
After receiving TOOL_RESULT: incorporate it into your answer.\n";

#[no_mangle]
pub fn main() {
    println("Hypha — Cellos AI agent (P2: file tools). Type 'exit' to quit.");

    let gw = match sys_spawn_from_path(GATEWAY_PATH) {
        SyscallResult::Ok(tid) => tid,
        _ => {
            println("[hypha] ERROR: cannot spawn llm-gateway");
            sys_exit(1);
        }
    };

    let tool_fs = match sys_spawn_from_path(TOOL_FS_PATH) {
        SyscallResult::Ok(tid) => tid,
        _ => {
            println("[hypha] WARN: tool-fs not found — file tools unavailable");
            0 // sentinel: 0 = no tool-fs; dispatch_tool will return an error
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

        match run_turn(gw, tool_fs, &prompt) {
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
    sys_exit(0);
}

/// Run one agentic turn: call the LLM, dispatch any tool requests, and return
/// the final text reply. Loops up to `MAX_TOOL_ROUNDS` times to handle chained
/// tool calls. Tool interactions are appended to `working_prompt` in-place
/// (not stored in the permanent conversation — only the final reply is kept).
fn run_turn(gw: usize, tool_fs: usize, prompt: &str) -> Result<String, String> {
    let mut working = String::from(prompt);
    for _ in 0..MAX_TOOL_ROUNDS {
        match ask(gw, &working)? {
            LlmReply::Text(t) => return Ok(t),
            LlmReply::ToolCalls(calls) => {
                for call in &calls {
                    print("[hypha] tool: ");
                    println(call.name.as_str());
                    let result = dispatch_tool(tool_fs, call)?;
                    working.push_str("\ntool_call: ");
                    working.push_str(&call.name);
                    working.push(' ');
                    working.push_str(&call.args_json);
                    working.push_str("\ntool_result: ");
                    working.push_str(&result);
                }
                working.push_str("\nassistant: ");
                working = trim_front(working, PROMPT_MAX);
            }
            LlmReply::Error(e) => return Err(e),
        }
    }
    Err(String::from("[tool limit reached — too many sequential calls]"))
}

/// One IPC round-trip with the gateway: send `LlmRequest`, receive `LlmReply`.
fn ask(gw: usize, prompt: &str) -> Result<LlmReply, String> {
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
            Ok(reply) => Ok(reply),
            Err(_) => Err(String::from("bad LlmReply encoding")),
        },
        _ => Err(String::from("no reply from gateway")),
    }
}

/// Dispatch one tool call to `tool-fs` via `AgentToolRequest` IPC.
fn dispatch_tool(tool_fs: usize, call: &ToolCall) -> Result<String, String> {
    if tool_fs == 0 {
        return Err(String::from("tool-fs not available"));
    }
    let req = AgentToolRequest::Invoke {
        name: &call.name,
        args_json: &call.args_json,
    };
    let mut out = [0u8; 4096];
    let encoded = postcard::to_slice(&req, &mut out)
        .map_err(|_| String::from("tool request too large"))?;

    match sys_send(tool_fs, encoded) {
        SyscallResult::Ok(_) => {}
        _ => return Err(String::from("send to tool-fs failed")),
    }

    let mut buf = [0u8; 4096];
    match sys_recv(0, &mut buf) {
        SyscallResult::Ok(_sender) => match postcard::from_bytes::<AgentToolResponse>(&buf) {
            Ok(AgentToolResponse::Ok { result_json }) => Ok(result_json),
            Ok(AgentToolResponse::Err { message }) => Err(message),
            Err(_) => Err(String::from("bad AgentToolResponse encoding")),
        },
        _ => Err(String::from("no reply from tool-fs")),
    }
}

/// Flatten the heap conversation into a single role-tagged transcript.
/// The system preamble is prepended so the LLM always knows about tools.
/// Trimmed from the front if it would exceed the one-message IPC budget.
fn render_prompt(conv: &[(&'static str, String)]) -> String {
    let mut s = String::from(SYSTEM_PREAMBLE);
    for (role, text) in conv {
        s.push_str(role);
        s.push_str(": ");
        s.push_str(text);
        s.push('\n');
    }
    s.push_str("assistant: ");
    trim_front(s, PROMPT_MAX)
}

/// Keep the tail of `s` within `max` bytes (drop oldest content), char-boundary safe.
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
