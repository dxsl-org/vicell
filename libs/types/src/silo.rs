// SPDX-License-Identifier: MPL-2.0
// Host-side protocol types for the Tier 3a Security Silo.
//
// The silo is a bare-metal AArch64 guest that holds a P-256 private key in
// Stage-2 fenced memory. The host communicates through a shared mailbox page
// mapped at IPA 0x4000_3000.  All constants here must stay byte-for-byte
// consistent with the guest-side `mailbox.rs`.

/// IPA of the 4 KiB mailbox page pre-mapped by the VMM before guest boot.
pub const MAILBOX_IPA: u64 = 0x4000_3000;

// ── HVC function IDs (SMCCC-style vendor range 0xC600_0080..0xC600_00FF) ──

/// Guest signals: key initialisation complete; response data = uncompressed pub key.
pub const HVC_SILO_READY: u64 = 0xC600_0080;
/// Guest signals: operation finished; response data in mailbox.
pub const HVC_SILO_DONE: u64 = 0xC600_0081;
/// Guest signals: unrecoverable error; mailbox data[0] = error code.
pub const HVC_SILO_FAULT: u64 = 0xC600_0082;

// ── Mailbox protocol ──────────────────────────────────────────────────────

/// Commands the host can send to the silo guest.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiloCmd {
    /// Key initialisation: `data[0..32]` = 32-byte entropy seed.
    Init = 0,
    /// Sign a pre-hashed digest: `data[0..32]` = SHA-256 digest.
    Sign = 1,
    /// Static-static ECDH: `data[0..65]` = peer uncompressed public key.
    Ecdh = 2,
    /// Return own public key; no input data required.
    GetPub = 3,
}

/// Response codes the silo guest writes back to the mailbox.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiloRespCode {
    /// Init succeeded: `data[0..65]` = uncompressed public key.
    Ready = 0,
    /// Sign succeeded: `data[0]` = DER length, `data[1..=len]` = DER signature.
    Signature = 1,
    /// ECDH succeeded: `data[0..32]` = raw shared secret.
    Secret = 2,
    /// GetPub succeeded: `data[0..65]` = uncompressed public key.
    PubKey = 3,
    /// Operation failed: `data[0]` = error code.
    Fault = 0xFF,
}

/// 4 KiB shared-memory page at `MAILBOX_IPA`.
///
/// The host writes `seq`, `cmd`, and `data`; then resumes the guest.
/// The guest reads the snapshot, processes it, writes `resp` + `data`, then
/// fires an HVC to signal completion.  The host must never touch the page
/// between guest resume and the HVC callback.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MailboxPage {
    /// Sequence number: host increments before each request; guest echoes it.
    pub seq: u32,
    /// Command discriminant — cast from `SiloCmd`.
    pub cmd: u8,
    /// Response code discriminant — written by guest, cast from `SiloRespCode`.
    pub resp: u8,
    pub _pad: [u8; 2],
    /// Payload: input data (host→guest) / output data (guest→host).
    pub data: [u8; 4088],
}

const _MAILBOX_SIZE_CHECK: () = assert!(core::mem::size_of::<MailboxPage>() == 4096);

// ── IPC wire format (host cell ↔ silo service cell) ──────────────────────

/// IPC request from a caller cell to the silo service cell (128 bytes total).
///
/// Fits in a single ViCell IPC message (≤ 4096 bytes; this is intentionally
/// small so it routes through the fast-path without a grant).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SiloRequest {
    /// Operation code — cast from `SiloCmd`.
    pub opcode: u8,
    pub _pad: [u8; 31],
    /// Payload: up to 65 bytes for an uncompressed P-256 public key (ECDH).
    pub data: [u8; 96],
}

/// IPC response from the silo service cell to the caller (128 bytes total).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SiloResponse {
    /// `0` = success, `0xFF` = fault.
    pub status: u8,
    /// Number of valid bytes in `data`.
    pub len: u8,
    pub _pad: [u8; 2],
    /// Result bytes: DER signature (≤ 72 B), shared secret (32 B), or pub key (65 B).
    pub data: [u8; 124],
}

const _SILO_REQ_SIZE_CHECK: () = assert!(core::mem::size_of::<SiloRequest>() == 128);
const _SILO_RESP_SIZE_CHECK: () = assert!(core::mem::size_of::<SiloResponse>() == 128);

/// Service registry ID for the silo service cell.
pub const SILO_SERVICE_ID: u16 = 6;
