//! Typed IPC message enums for ViCell services.
//!
//! Both kernel and Cell crates link to `libs/api`, so these types are shared
//! across the IPC boundary without any unsafe casting.
//!
//! ## Wire format
//! `postcard::to_slice` serializes into a caller-provided `[u8; IPC_BUF_SIZE]`
//! stack buffer.  The receiver calls `postcard::from_bytes` to deserialize.
//! Discriminants are 1-byte varints (< 127 variants); slice lengths are varint-
//! prefixed.  Total overhead per message is minimal compared to the ecall trap.
//!
//! ## Lifetime contract
//! Types that borrow (`&'a str`, `&'a [u8]`) point into the caller's buffer.
//! The decoded value MUST be consumed before the buffer is reused.

use serde::{Deserialize, Serialize};

/// Fixed IPC payload size matching the existing kernel IPC convention.
pub const IPC_BUF_SIZE: usize = 512;

// ── VFS service ───────────────────────────────────────────────────────────────

/// Requests sent to the VFS service (`/bin/vfs`).
#[derive(Debug, Serialize, Deserialize)]
pub enum VfsRequest<'a> {
    /// Read file content from a VFS-managed path.
    GetFile(&'a str),
    /// List directory entries as a newline-separated string.
    ListDir(&'a str),
    /// Stat a path — returns size + is_dir flag.
    Stat(&'a str),
    /// Write (create/overwrite) a file.
    Write { path: &'a str, content: &'a [u8] },
    /// Append bytes to an existing file (or create it).
    Append { path: &'a str, content: &'a [u8] },
    /// Create a directory.
    Mkdir(&'a str),
    /// Remove an empty directory.
    Rmdir(&'a str),
    /// Delete a file.
    Unlink(&'a str),
    /// Remove a directory tree recursively.
    RmdirRecursive(&'a str),
}

/// Responses from the VFS service.
#[derive(Debug, Serialize, Deserialize)]
pub enum VfsResponse<'a> {
    /// File or directory listing bytes (copied into response buffer).
    Data(&'a [u8]),
    /// Zero-copy file access: raw pointer + length in the shared address space.
    /// Used by binary loaders that need to map the ELF directly from VFS memory.
    DataPtr { ptr: u64, len: u64 },
    /// Stat result: (file_size, is_dir).
    Stat { size: u64, is_dir: bool },
    /// Successful write, mkdir, unlink, etc.
    Ok,
    /// Error — opaque error code (`types::ViError` discriminant).
    Err(u8),
}

// ── Network service ───────────────────────────────────────────────────────────

/// Requests sent to the network service (`/bin/net`).
#[derive(Debug, Serialize, Deserialize)]
pub enum NetRequest<'a> {
    TcpConnect  { addr: [u8; 4], port: u16 },
    TcpSend     { cap_id: u32, data: &'a [u8] },
    TcpRecv     { cap_id: u32, buf_len: u32 },
    TcpClose    { cap_id: u32 },
    TcpListen   { port: u16 },
    TcpAccept   { cap_id: u32 },
    UdpCreate,
    UdpSend     { cap_id: u32, addr: [u8; 4], port: u16, data: &'a [u8] },
    UdpRecv     { cap_id: u32, buf_len: u32 },
    Resolve     { hostname: &'a str },
    SocketState { cap_id: u32 },
}

/// Responses from the network service.
#[derive(Debug, Serialize, Deserialize)]
pub enum NetResponse<'a> {
    CapId  (u32),
    Data   (&'a [u8]),
    Addr   ([u8; 4]),
    State  (u8),
    Ok,
    Err    (u8),
}

// ── Serialization helpers ─────────────────────────────────────────────────────

/// Serialize `msg` into `buf`.
///
/// Returns a slice of the bytes actually written.
///
/// # Errors
/// Returns `postcard::Error` if the encoded message exceeds `buf.len()`.
pub fn encode<'a, T: Serialize>(msg: &T, buf: &'a mut [u8]) -> postcard::Result<&'a mut [u8]> {
    postcard::to_slice(msg, buf)
}

/// Deserialize a typed message from the START of `buf`, tolerating trailing bytes.
///
/// The IPC receive buffer is fixed at 512 bytes; the sender may have encoded a
/// shorter message.  `take_from_bytes` reads exactly what the message needs and
/// ignores the rest — unlike `from_bytes` which requires the entire slice to be
/// consumed.
///
/// For types with borrowed fields (`&str`, `&[u8]`), the returned value borrows
/// from `buf` — it must be consumed before `buf` is reused or overwritten.
pub fn decode<'de, T: Deserialize<'de>>(buf: &'de [u8]) -> postcard::Result<T> {
    let (msg, _remaining) = postcard::take_from_bytes(buf)?;
    Ok(msg)
}
