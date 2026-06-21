//! Signed operator policy (roadmap §G.2 P5b) — the headless "consent" mechanism.
//!
//! At boot the kernel reads `/POLICY.BIN` from the kernel-embedded VIFS1, verifies
//! its Ed25519 signature against the fleet root public key, and parses it into a
//! `path → CapSet` table. Phase 04 folds `lookup()` into the spawn-time grant so
//! the effective caps are `manifest ∩ spawner ∩ policy`.
//!
//! Security invariants (red-team-driven):
//! - **Verify-then-parse:** the signature covers `blob[..len-64]`; verify FIRST
//!   (length-only, no field parsing) so the parser never runs on unverified bytes.
//! - **Panic-free parser:** every field read is bounds-checked; malformed →
//!   `Invalid`, never a panic (a boot-path panic = no boot = bricked robot).
//! - **Fail-safe:** an *invalid* signature/parse is ALWAYS fail-closed. An *absent*
//!   policy is dev-permissive in G1 (this build) and fail-closed only when the
//!   `policy-required` feature is set (real-fleet posture). See `lookup`.
//! - **Domain validation:** parsed cap bytes are masked to known bits; unknown
//!   bits → `Invalid` (a signed-but-malformed policy is still rejected).

use crate::resource_registry::{DEV_GPIO, DEV_UART};
use crate::sync::Spinlock;
use crate::task::cap::CapSet;
use alloc::string::String;
use alloc::vec::Vec;

/// Magic "VPOL" as a little-endian u32 (bytes V,P,O,L).
const MAGIC: u32 = u32::from_le_bytes([b'V', b'P', b'O', b'L']);
const VERSION: u8 = 1;
const SIG_LEN: usize = 64;
const HEADER_LEN: usize = 8; // magic(4) + version(1) + flags(1) + entry_count(2)
const CAP_BYTES: usize = 6; // block_io, network, spawn, hyp, mmio_devices, block_regions
/// 8.3-safe, root-level path (VIFS1 uppercases + is FAT16 8.3).
const POLICY_PATH: &str = "/POLICY.BIN";

/// Valid `mmio_devices` bits and `block_regions` bits (domain-validation masks).
const MMIO_MASK: u8 = DEV_GPIO | DEV_UART;
const REGION_MASK: u8 = 0b111;

/// Dev fleet Ed25519 **public** key — derived from the fixed dev seed in
/// `scripts/sign-policy.py` (reproducible; a dev key, never shipped in release).
const DEV_FLEET_PUBKEY: [u8; 32] = [
    0x21, 0x52, 0xf8, 0xd1, 0x9b, 0x79, 0x1d, 0x24, 0x45, 0x32, 0x42, 0xe1, 0x5f, 0x2e, 0xab, 0x6c,
    0xb7, 0xcf, 0xfa, 0x7b, 0x6a, 0x5e, 0xd3, 0x00, 0x97, 0x96, 0x0e, 0x06, 0x98, 0x81, 0xdb, 0x12,
];

/// Fleet root Ed25519 **public** key (trust anchor; lives in the kernel TCB, not
/// in mutable VIFS1 data). `dev-policy-key` feature → the dev key (so a dev-signed
/// `/POLICY.BIN` verifies); otherwise a placeholder the production provisioning
/// replaces. A zero/placeholder key fails every verify → any present policy is
/// `Invalid` (fail-closed), the safe direction; absent policy still boots
/// (dev-permissive).
#[cfg(feature = "dev-policy-key")]
const FLEET_ROOT_PUBKEY: [u8; 32] = DEV_FLEET_PUBKEY;
#[cfg(not(feature = "dev-policy-key"))]
const FLEET_ROOT_PUBKEY: [u8; 32] = [0u8; 32]; // TODO(prod): provisioned fleet key

/// Result of a policy lookup for a given cell path.
pub enum PolicyDecision {
    /// Policy explicitly grants this path the given caps (ceiling).
    Permit(CapSet),
    /// Policy is present and explicitly denies (or invalid → fail-closed).
    DenyAll,
    /// No policy entry for this path (or policy absent). Caller applies the
    /// fail-safe rule: dev-permissive keeps the spawner-intersected caps;
    /// `policy-required` treats it as deny.
    NoEntry,
}

struct PolicyEntry {
    path: String,
    caps: CapSet,
}

enum PolicyState {
    Loaded(Vec<PolicyEntry>),
    Absent,
    Invalid,
}

static POLICY: Spinlock<Option<PolicyState>> = Spinlock::new(None);

/// Force-release this module's lock during fault teardown.
///
/// # Safety
/// Single-hart; called only from the fault/panic path with interrupts disabled.
pub unsafe fn force_unlock_locks() {
    POLICY.force_unlock();
}

/// Load + verify the operator policy from VIFS1. Call once at boot AFTER
/// `fs::init()` and BEFORE the first cap-bearing cell spawns. Eager-only (no
/// lazy path — VIFS1 is kernel-embedded and available this early).
pub fn load_from_vifs1() {
    let blob = match crate::fs::read_file_from_vifs1(POLICY_PATH) {
        Ok(b) if !b.is_empty() => b,
        _ => {
            log::info!("[policy] no {} in VIFS1 — absent", POLICY_PATH);
            crate::audit::log_event(crate::audit::AuditEvent::PolicyAbsent, &crate::audit::encode_u32x2(0, 0));
            *POLICY.lock() = Some(PolicyState::Absent);
            return;
        }
    };

    // Verify-then-parse: the trailing SIG_LEN bytes are the signature over the body.
    if blob.len() < HEADER_LEN + SIG_LEN {
        return mark_invalid(1);
    }
    let split = blob.len() - SIG_LEN;
    let (body, sig) = blob.split_at(split);
    let mut sig64 = [0u8; SIG_LEN];
    sig64.copy_from_slice(sig);
    if !crate::ed25519::verify(&FLEET_ROOT_PUBKEY, body, &sig64) {
        log::warn!("[policy] signature verification FAILED — fail-closed");
        return mark_invalid(2);
    }

    match parse(body) {
        Some(entries) => {
            let n = entries.len() as u32;
            log::info!("[policy] loaded + verified ({} entries)", n);
            crate::audit::log_event(crate::audit::AuditEvent::PolicyLoaded, &crate::audit::encode_u32x2(n, 0));
            *POLICY.lock() = Some(PolicyState::Loaded(entries));
        }
        None => {
            log::warn!("[policy] malformed body — fail-closed");
            mark_invalid(3);
        }
    }
}

fn mark_invalid(reason: u32) {
    crate::audit::log_event(crate::audit::AuditEvent::PolicyInvalid, &crate::audit::encode_u32x2(reason, 0));
    *POLICY.lock() = Some(PolicyState::Invalid);
}

/// Parse the (already signature-verified) body into entries. Panic-free: every
/// read is bounds-checked; any malformation or out-of-domain cap bit → `None`.
fn parse(body: &[u8]) -> Option<Vec<PolicyEntry>> {
    if body.len() < HEADER_LEN {
        return None;
    }
    let magic = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if magic != MAGIC || body[4] != VERSION {
        return None;
    }
    let count = u16::from_le_bytes([body[6], body[7]]) as usize;

    let mut entries = Vec::new();
    let mut off = HEADER_LEN;
    for _ in 0..count {
        // path_len
        let path_len = *body.get(off)? as usize;
        off += 1;
        // path bytes
        let path_bytes = body.get(off..off.checked_add(path_len)?)?;
        let path = core::str::from_utf8(path_bytes).ok()?;
        off += path_len;
        // 6 cap bytes
        let caps_raw = body.get(off..off.checked_add(CAP_BYTES)?)?;
        off += CAP_BYTES;

        let mmio_devices = caps_raw[4];
        let block_regions = caps_raw[5];
        // Domain validation: reject unknown bits (signed-but-malformed).
        if mmio_devices & !MMIO_MASK != 0 || block_regions & !REGION_MASK != 0 {
            return None;
        }
        entries.push(PolicyEntry {
            path: String::from(path),
            caps: CapSet {
                block_io: caps_raw[0] != 0,
                network: caps_raw[1] != 0,
                spawn: caps_raw[2] != 0,
                hypervisor: caps_raw[3] != 0,
                mmio_devices,
                block_regions,
            },
        });
    }
    Some(entries)
}

/// Self-test of the full signed-policy path: verify + parse a known dev-signed
/// blob (from `scripts/sign-policy.py`), confirm a known entry parses correctly,
/// and confirm a tampered blob is REJECTED. Returns `true` iff both hold. Run as
/// a boot power-on self-test before trusting the policy path.
pub fn self_test() -> bool {
    // 135-byte dev-signed blob (4 entries) emitted by scripts/sign-policy.py.
    const BLOB: [u8; 135] = [
        0x56, 0x50, 0x4f, 0x4c, 0x01, 0x00, 0x04, 0x00, 0x08, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x76, 0x66,
        0x73, 0x01, 0x00, 0x00, 0x00, 0x00, 0x07, 0x08, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x6e, 0x65, 0x74,
        0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x73, 0x68, 0x65, 0x6c,
        0x6c, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x09, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x69, 0x6e, 0x69,
        0x74, 0x01, 0x01, 0x01, 0x00, 0x03, 0x07, 0x44, 0x17, 0x69, 0xc2, 0xc9, 0x40, 0x3a, 0x1f, 0x67,
        0xcf, 0xfa, 0x4d, 0xa1, 0x23, 0x15, 0x29, 0xa1, 0xa6, 0x62, 0x9f, 0xb4, 0xde, 0x48, 0xe1, 0x61,
        0x00, 0x0f, 0x83, 0x98, 0x01, 0x00, 0x46, 0x06, 0x6d, 0x20, 0xa8, 0xa5, 0xff, 0xd9, 0x05, 0x4f,
        0x51, 0x12, 0x46, 0xc6, 0x45, 0x59, 0x7b, 0x15, 0xae, 0x1e, 0x22, 0xb6, 0x33, 0xb4, 0x2b, 0xc8,
        0x84, 0x28, 0x2d, 0x83, 0x7f, 0xde, 0x00,
    ];
    if BLOB.len() < HEADER_LEN + SIG_LEN {
        return false;
    }
    let (body, sig) = BLOB.split_at(BLOB.len() - SIG_LEN);
    let mut s = [0u8; SIG_LEN];
    s.copy_from_slice(sig);

    // 1. Valid blob: signature verifies + parses + /bin/vfs has the expected caps.
    if !crate::ed25519::verify(&DEV_FLEET_PUBKEY, body, &s) {
        return false;
    }
    let Some(entries) = parse(body) else { return false; };
    let Some(vfs) = entries.iter().find(|e| e.path == "/bin/vfs") else { return false; };
    if !vfs.caps.block_io || vfs.caps.block_regions != 0b111 {
        return false;
    }
    // 2. Tampered blob: a flipped body byte must FAIL verification.
    let mut bad = BLOB;
    bad[10] ^= 0x01;
    let (bad_body, _) = bad.split_at(bad.len() - SIG_LEN);
    if crate::ed25519::verify(&DEV_FLEET_PUBKEY, bad_body, &s) {
        return false;
    }
    true
}

/// Policy decision for a cell path. See `PolicyDecision`; the caller (Phase 04)
/// applies the dev-permissive vs `policy-required` fail-safe rule to `NoEntry`.
pub fn lookup(path: &str) -> PolicyDecision {
    let guard = POLICY.lock();
    match guard.as_ref() {
        Some(PolicyState::Loaded(entries)) => {
            for e in entries {
                if e.path == path {
                    return PolicyDecision::Permit(e.caps);
                }
            }
            PolicyDecision::NoEntry
        }
        // Invalid → fail-closed ALWAYS, regardless of posture.
        Some(PolicyState::Invalid) => PolicyDecision::DenyAll,
        // Absent / not-yet-loaded → NoEntry; the caller's fail-safe rule decides
        // (dev-permissive keeps caps; `policy-required` denies).
        Some(PolicyState::Absent) | None => PolicyDecision::NoEntry,
    }
}
