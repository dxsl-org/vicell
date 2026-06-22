//! Shared IPC contract types for the **Hypha** AI agent.
//!
//! Three boundaries:
//! - `core` ↔ `llm-gateway`: [`LlmRequest`] / [`LlmReply`]
//! - `core` ↔ tool Cells:    [`AgentToolRequest`] / [`AgentToolResponse`]
//!
//! Serialized with `postcard` at the call sites (same convention as
//! `libs/api` ipc types). Borrowed fields (`&'a str`) point into the caller's
//! IPC buffer and must be consumed before the buffer is reused.

#![no_std]

extern crate alloc;

use alloc::string::String;
use serde::{Deserialize, Serialize};

/// Temporary Hypha-private service id for the llm-gateway, used until the
/// kernel registry gains dynamic/name-based discovery (os-gap G7). Well-known
/// kernel service ids are `1..=5` (VFS..COMPOSITOR); a high value avoids any
/// collision with those.
pub const HYPHA_LLM_SERVICE: u16 = 64;

/// `core` → `llm-gateway`.
///
/// P0 carries the prompt **inline** (small prompts fit the 4 KiB IPC buffer).
/// Large prompts move to a Grant reference in a later phase (os-gap G5).
#[derive(Debug, Serialize, Deserialize)]
pub enum LlmRequest<'a> {
    Complete { prompt: &'a str },
}

/// `llm-gateway` → `core`.
#[derive(Debug, Serialize, Deserialize)]
pub enum LlmReply {
    Text(String),
    /// LLM wants to invoke one or more tools (P2+). The gateway sets this when
    /// the content starts with `TOOL_CALL:`.
    ToolCalls(alloc::vec::Vec<ToolCall>),
    Error(String),
}

/// A tool invocation requested by the model. Used from P2; defined now to lock
/// the contract. The model speaks JSON, so args ride as a JSON string.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub args_json: String,
}

/// `core` → tool Cell.
#[derive(Debug, Serialize, Deserialize)]
pub enum AgentToolRequest<'a> {
    Invoke { name: &'a str, args_json: &'a str },
}

/// tool Cell → `core`.
#[derive(Debug, Serialize, Deserialize)]
pub enum AgentToolResponse {
    Ok { result_json: String },
    Err { message: String },
}
