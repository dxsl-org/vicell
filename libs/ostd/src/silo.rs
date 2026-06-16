// SPDX-License-Identifier: MPL-2.0
//! Security Silo client handle.
//!
//! Provides a zero-knowledge IPC wrapper over the silo service cell.
//! Callers never see the P-256 private key — only its public key,
//! signatures, and ECDH shared secrets.
//!
//! # Protocol
//! All requests are 128-byte raw buffers (no postcard):
//! - `req[0]`       = `SiloCmd` discriminant
//! - `req[1..32]`   = zero padding
//! - `req[32..128]` = payload (up to 96 bytes)
//!
//! All responses are 128-byte raw buffers:
//! - `resp[0]`      = status (`0` = ok, `0xFF` = fault)
//! - `resp[1]`      = valid byte count in `resp[4..]`
//! - `resp[2..4]`   = zero padding
//! - `resp[4..128]` = result bytes (DER sig ≤ 72 B, secret 32 B, or pub key 65 B)

extern crate alloc;

use types::silo::{SiloCmd, SILO_SERVICE_ID};
use crate::{syscall, task};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors returned by [`SiloHandle`] operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiloError {
    /// Silo service is not registered (not yet spawned or crashed).
    ServiceNotFound,
    /// The silo guest returned a fault status code (0xFF).
    GuestFault,
    /// The IPC receive returned an unexpected zero sender — no message delivered.
    IpcError,
}

// ── DER signature wrapper ─────────────────────────────────────────────────────

/// A DER-encoded P-256 ECDSA signature (maximum 72 bytes).
#[derive(Clone, Copy)]
pub struct SigDer {
    pub bytes: [u8; 72],
    /// Number of valid bytes in `bytes`.
    pub len: u8,
}

// ── SiloHandle ────────────────────────────────────────────────────────────────

/// Client handle to the Security Silo service cell.
///
/// Resolves the silo TID once on [`connect`][Self::connect] and caches it.
/// Each method is one synchronous IPC round-trip. The handle is not `Send`;
/// callers that need cross-task access must use external serialization.
pub struct SiloHandle {
    tid: usize,
}

impl SiloHandle {
    /// Resolve the silo service TID, retrying up to 8 times with scheduler yields.
    ///
    /// Returns `Err(SiloError::ServiceNotFound)` if the silo service is not yet
    /// registered after all retries — call again after a short delay.
    pub fn connect() -> Result<Self, SiloError> {
        for _ in 0..8 {
            if let Some(tid) = syscall::sys_lookup_service(SILO_SERVICE_ID) {
                return Ok(Self { tid });
            }
            task::yield_now();
        }
        Err(SiloError::ServiceNotFound)
    }

    /// Return the cached silo service TID (for diagnostic use only).
    #[inline]
    pub fn tid(&self) -> usize {
        self.tid
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    /// Build a 128-byte request buffer.
    ///
    /// `opcode` → `[0]`; payload bytes → `[32..32+min(payload.len(), 96)]`.
    fn build_req(opcode: SiloCmd, payload: &[u8]) -> [u8; 128] {
        let mut req = [0u8; 128];
        req[0] = opcode as u8;
        let copy_len = payload.len().min(96);
        req[32..32 + copy_len].copy_from_slice(&payload[..copy_len]);
        req
    }

    /// Send a 128-byte raw request and receive a 128-byte raw response.
    ///
    /// Sequence contract: `sys_send` fires synchronously; `sys_recv(0, …)`
    /// blocks until the silo service replies. The silo service processes
    /// one request at a time (FIFO), so the response always belongs to this
    /// call as long as the caller serializes access.
    fn call(&self, req: &[u8; 128]) -> Result<[u8; 128], SiloError> {
        // sys_send always returns Ok — the kernel never rejects a well-formed
        // send to a live TID via this ABI path.
        let _ = syscall::sys_send(self.tid, req);

        let mut resp = [0u8; 128];
        match syscall::sys_recv(0, &mut resp) {
            // sender > 0 means a real message arrived.
            syscall::SyscallResult::Ok(sender) if sender > 0 => {}
            // Ok(0) is a non-blocking fast-path "no message" return; treat as error.
            _ => return Err(SiloError::IpcError),
        }

        if resp[0] == 0xFF {
            return Err(SiloError::GuestFault);
        }
        Ok(resp)
    }

    // ── Public API ─────────────────────────────────────────────────────────

    /// Seed the silo's P-256 private key from 32 bytes of entropy.
    ///
    /// Must be called exactly once after [`connect`][Self::connect]. Returns
    /// the silo's uncompressed SEC1 public key (65 bytes) so the caller can
    /// persist it for verification.
    ///
    /// # Security
    /// The seed is forwarded to the silo guest through the mailbox page and
    /// used immediately to derive the key. The guest zeros the seed slot.
    /// Do not pass a seed derived from a low-entropy source.
    pub fn init_key(&self, seed: &[u8; 32]) -> Result<[u8; 65], SiloError> {
        let req = Self::build_req(SiloCmd::Init, seed);
        let resp = self.call(&req)?;
        let len = resp[1] as usize;
        if len != 65 {
            return Err(SiloError::GuestFault);
        }
        let mut pub_key = [0u8; 65];
        pub_key.copy_from_slice(&resp[4..69]);
        Ok(pub_key)
    }

    /// Retrieve the silo's current P-256 public key (65 bytes, uncompressed SEC1).
    pub fn get_public_key(&self) -> Result<[u8; 65], SiloError> {
        let req = Self::build_req(SiloCmd::GetPub, &[]);
        let resp = self.call(&req)?;
        let len = resp[1] as usize;
        if len != 65 {
            return Err(SiloError::GuestFault);
        }
        let mut pub_key = [0u8; 65];
        pub_key.copy_from_slice(&resp[4..69]);
        Ok(pub_key)
    }

    /// Sign a pre-hashed SHA-256 digest inside the silo.
    ///
    /// Returns the DER-encoded P-256 ECDSA signature (≤ 72 bytes). The private
    /// key never leaves the Stage-2 fence.
    pub fn sign(&self, digest: &[u8; 32]) -> Result<SigDer, SiloError> {
        let req = Self::build_req(SiloCmd::Sign, digest);
        let resp = self.call(&req)?;
        let len = resp[1] as usize;
        if len == 0 || len > 72 {
            return Err(SiloError::GuestFault);
        }
        let mut sig = SigDer { bytes: [0u8; 72], len: len as u8 };
        sig.bytes[..len].copy_from_slice(&resp[4..4 + len]);
        Ok(sig)
    }

    /// Perform static ECDH with a peer's uncompressed P-256 public key (65 bytes).
    ///
    /// Returns the 32-byte raw shared secret. The local private key never leaves
    /// the Stage-2 fence.
    pub fn ecdh(&self, peer_pub: &[u8; 65]) -> Result<[u8; 32], SiloError> {
        let req = Self::build_req(SiloCmd::Ecdh, peer_pub);
        let resp = self.call(&req)?;
        let len = resp[1] as usize;
        if len != 32 {
            return Err(SiloError::GuestFault);
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&resp[4..36]);
        Ok(secret)
    }

    /// Send a raw request by numeric opcode (for fault-recovery testing — T5).
    ///
    /// `opcode` → `req[0]`; `payload[..min(len, 96)]` → `req[32..]`.
    /// Returns `Err(GuestFault)` on silo fault, `Err(IpcError)` on IPC failure.
    pub fn send_raw(&self, opcode: u8, payload: &[u8]) -> Result<[u8; 128], SiloError> {
        let mut req = [0u8; 128];
        req[0] = opcode;
        let copy_len = payload.len().min(96);
        req[32..32 + copy_len].copy_from_slice(&payload[..copy_len]);
        self.call(&req)
    }
}
