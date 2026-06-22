//! Minimal HTTP/1.0 + JSON helpers for P0/P2.
//!
//! Hand-rolled on purpose (os-gaps **G1** no HTTP lib, **G4** no_std JSON):
//! good enough to prove the LLM round-trip. Promote to `ostd::http` +
//! `serde-json-core` once a second consumer appears or parsing needs to be
//! robust against arbitrary provider responses.

use agent_proto::ToolCall;
use alloc::format;
use alloc::string::String;

/// Build an OpenAI-compatible chat-completion JSON body with a single user turn.
pub fn build_chat_body(model: &str, prompt: &str) -> String {
    format!(
        "{{\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}]}}",
        json_escape(model),
        json_escape(prompt),
    )
}

/// Build an HTTP/1.0 POST with `Connection: close` (so the server closes the
/// stream when the body is done — our read loop relies on that).
pub fn build_post(host: &str, path: &str, body: &str) -> String {
    format!(
        "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        host,
        body.len(),
        body,
    )
}

/// Escape a string for use inside a JSON string literal.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Return the body slice after the first `\r\n\r\n` header terminator.
pub fn http_body(resp: &[u8]) -> Option<&[u8]> {
    resp.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| &resp[i + 4..])
}

/// If `content` starts with `TOOL_CALL:` (ReAct-style), parse and return a
/// [`ToolCall`]. Returns `None` for ordinary text replies.
///
/// Expected format (LLM must emit ONLY this, nothing else):
/// `TOOL_CALL: {"name":"tool_name","args":{...}}`
pub fn extract_tool_call(content: &str) -> Option<ToolCall> {
    let s = content.trim_start_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r');
    let rest = s.strip_prefix("TOOL_CALL:")?;
    let json = rest.trim_start_matches(|c: char| c == ' ' || c == '\t');
    let name = json_extract_str(json, "name")?;
    let args_json = json_extract_obj(json, "args").unwrap_or("{}");
    Some(ToolCall {
        name: String::from(name),
        args_json: String::from(args_json),
    })
}

/// Extract a plain JSON string field from a flat JSON object: `"key": "value"`.
/// Does not handle nested objects inside the value, but handles `\"` escapes.
fn json_extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("\"{}\"", key);
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
            idx += 1; // skip escaped character
        }
        idx += 1;
    }
    Some(&json[start..idx])
}

/// Extract a JSON object value `{...}` for a given key using brace counting.
/// Handles nested objects and quoted strings containing braces.
fn json_extract_obj<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("\"{}\"", key);
    let mut idx = json.find(search.as_str())? + search.len();
    let bytes = json.as_bytes();
    while idx < bytes.len() && matches!(bytes[idx], b' ' | b'\t' | b':') {
        idx += 1;
    }
    if idx >= bytes.len() || bytes[idx] != b'{' {
        return None;
    }
    let start = idx;
    let mut depth = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&json[start..=idx]);
                }
            }
            b'"' => {
                idx += 1;
                while idx < bytes.len() && bytes[idx] != b'"' {
                    if bytes[idx] == b'\\' {
                        idx += 1;
                    }
                    idx += 1;
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

/// Extract the first `"content": "..."` string value from a JSON body and
/// unescape it. P0-grade: finds the key then reads the following JSON string.
/// Replace with a real parser (os-gap G4) for nested/duplicate-key safety.
pub fn extract_content(body: &[u8]) -> Option<String> {
    let s = core::str::from_utf8(body).ok()?;
    let key = "\"content\"";
    let mut idx = s.find(key)? + key.len();
    let bytes = s.as_bytes();
    while idx < bytes.len() && (bytes[idx] == b' ' || bytes[idx] == b':') {
        idx += 1;
    }
    if idx >= bytes.len() || bytes[idx] != b'"' {
        return None;
    }
    idx += 1; // past the opening quote

    let mut out = String::new();
    let mut chars = s[idx..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000C}'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'u' => {
                    let mut code = 0u32;
                    for _ in 0..4 {
                        code = code * 16 + chars.next()?.to_digit(16)?;
                    }
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}
