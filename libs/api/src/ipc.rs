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

/// IPC payload buffer size.  Must be large enough for the largest serialized
/// message in the system — currently a VFS Write with ~900-byte content
/// (~916 bytes encoded).  4 KiB gives comfortable headroom.
pub const IPC_BUF_SIZE: usize = 4096;

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
    /// Start a non-blocking file read.  Returns `PendingHandle(id)` immediately;
    /// call `Poll { handle: id }` to retrieve data when ready.
    ReadAsync { path: &'a str },
    /// Poll a pending read for completion.
    Poll { handle: u32 },
    /// Zero-copy large read: VFS reads `size` bytes at `offset` from the file
    /// identified by `cap` directly into the caller's pre-allocated grant buffer.
    /// Grant must be owned by the caller and large enough to hold `size` bytes.
    /// VFS calls GrantShare on itself before writing, then replies GrantDone.
    ReadGrant { cap: u64, offset: u64, size: usize, grant: usize },
    /// Zero-copy large write: VFS reads `bytes` bytes from the caller's grant
    /// buffer and writes them to `cap` at `offset`.  GrantDone is sent only
    /// after the write commits (write-through on FAT32) — F14 invariant.
    WriteGrant { cap: u64, offset: u64, grant: usize, bytes: usize },
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
    /// Async read accepted; poll this handle for completion.
    PendingHandle(u32),
    /// Read still in progress — call Poll again after yielding.
    Pending,
    /// Zero-copy I/O complete: `bytes` is the number of bytes transferred.
    GrantDone { bytes: usize },
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
    /// Bind a UDP socket to a local port.  Port 0 = auto-assign ephemeral.
    UdpBind         { cap_id: u32, port: u16 },
    /// Return the DHCP-assigned local IPv4 address.
    GetLocalIp,
    /// Join an IPv4 multicast group (IGMP); `cap_id` is unused (iface-level).
    MulticastJoin   { cap_id: u32, group: [u8; 4] },
    /// Leave a previously joined IPv4 multicast group.
    MulticastLeave  { cap_id: u32, group: [u8; 4] },
    /// Forward a raw L2 Ethernet frame to the NIC TX.
    /// `data` is the raw frame bytes (up to 1514 B, no virtio_net_hdr prefix).
    /// The Net Cell calls `sys_net_tx(data)` directly — smoltcp is bypassed.
    L2Send { data: &'a [u8] },
    /// Poll for one inbound L2 frame from the NIC addressed to `guest_mac`.
    /// The Net Cell splits RX frames by dst MAC; returns `Data(frame)` or `Ok` (empty).
    L2Recv { guest_mac: [u8; 6] },
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

// ── Input service ─────────────────────────────────────────────────────────────

/// Requests sent to the input service (`/bin/input`).
#[derive(Debug, Serialize, Deserialize)]
pub enum InputRequest {
    /// Register the caller as the currently focused cell that receives key/mouse events.
    SetFocus { cell_tid: u32 },
    /// Query which cell currently has focus.  Returns `InputResponse::Focus`.
    GetFocus,
    /// Unregister `cell_tid` from receiving events.  No-op if not currently focused.
    ClearFocus { cell_tid: u32 },
}

/// Responses from the input service.
#[derive(Debug, Serialize, Deserialize)]
pub enum InputResponse {
    /// Currently focused cell tid (0 = none).
    Focus(u32),
    Ok,
    Err(u8),
}

// ── Config service ────────────────────────────────────────────────────────────

/// Requests sent to the config service (`/bin/config`).
#[derive(Debug, Serialize, Deserialize)]
pub enum ConfigRequest<'a> {
    /// Read the value for `key`.  Returns `ConfigResponse::Value` or `NotFound`.
    Get(&'a str),
    /// Write `value` for `key` (insert or overwrite).  Returns `Ok`.
    Set { key: &'a str, value: &'a str },
    /// Remove `key`.  Returns `Ok` even if absent (idempotent).
    Delete(&'a str),
    /// List all registered keys as a newline-separated string.
    List,
}

/// Responses from the config service.
#[derive(Debug, Serialize, Deserialize)]
pub enum ConfigResponse<'a> {
    /// Value associated with a key.
    Value(&'a str),
    /// Newline-separated list of all keys.
    Keys(&'a str),
    Ok,
    NotFound,
    Err(u8),
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
