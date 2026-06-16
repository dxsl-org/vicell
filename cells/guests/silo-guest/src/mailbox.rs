// SPDX-License-Identifier: MPL-2.0
// Mailbox protocol types for the silo guest.
//
// This is the guest-side mirror of `libs/types/src/silo.rs`.  It is
// duplicated here because the guest is a bare-metal binary that cannot depend
// on the ViCell `libs/` tree (different target, no syscalls).
//
// Invariant: constants and layout must stay byte-for-byte in sync with the
// host-side `silo.rs`.

#![allow(dead_code)]

/// IPA of the 4 KiB mailbox page pre-mapped by the VMM at guest boot.
///
/// The guest must never store a local copy of this pointer — always call
/// `read_mailbox()` to get a stack snapshot (TOCTOU guard).
pub const MAILBOX_IPA: *mut MailboxPage = 0x4000_3000usize as *mut MailboxPage;

// ── HVC function IDs ─────────────────────────────────────────────────────────

pub const HVC_SILO_READY: u64 = 0xC600_0080;
pub const HVC_SILO_DONE: u64 = 0xC600_0081;
pub const HVC_SILO_FAULT: u64 = 0xC600_0082;

// ── Mailbox command discriminants ────────────────────────────────────────────

/// Commands the host sends to the guest.
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SiloCmd {
    Init = 0,
    Sign = 1,
    Ecdh = 2,
    GetPub = 3,
    /// Catch-all for unrecognised bytes — guest responds with Fault.
    Unknown = 0xFF,
}

impl From<u8> for SiloCmd {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Init,
            1 => Self::Sign,
            2 => Self::Ecdh,
            3 => Self::GetPub,
            _ => Self::Unknown,
        }
    }
}

// ── Mailbox page layout ──────────────────────────────────────────────────────

/// 4 KiB shared-memory page at `MAILBOX_IPA`.
///
/// Layout contract (must match host `MailboxPage`):
/// - Bytes 0..3   : `seq` (u32 LE)
/// - Byte 4       : `cmd`
/// - Byte 5       : `resp`
/// - Bytes 6..7   : padding
/// - Bytes 8..4095: `data`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MailboxPage {
    pub seq: u32,
    pub cmd: u8,
    pub resp: u8,
    pub _pad: [u8; 2],
    pub data: [u8; 4088],
}

// Compile-time layout check.
const _: () = assert!(core::mem::size_of::<MailboxPage>() == 4096);

// ── Mailbox accessors ────────────────────────────────────────────────────────

/// Copy the mailbox page to the stack.
///
/// # Safety
/// The caller must ensure the VMM has resumed the guest before this is called
/// (i.e. the host has completed its write to the IPA).  The volatile copy
/// prevents the compiler from caching a stale value.
pub unsafe fn read_mailbox() -> MailboxPage {
    // SAFETY: MAILBOX_IPA is a valid 4KiB page pre-mapped by the VMM.
    // volatile ensures we see the host's latest write.
    core::ptr::read_volatile(MAILBOX_IPA)
}

/// Write the response page back to the mailbox IPA.
///
/// # Safety
/// Must be called exactly once per request, before firing the HVC signal.
pub unsafe fn write_mailbox(page: &MailboxPage) {
    // SAFETY: same region as read_mailbox; volatile prevents store elision.
    core::ptr::write_volatile(MAILBOX_IPA, *page);
}

/// Fire an HVC to signal the host.
///
/// `func_id` must be one of `HVC_SILO_*`.  The host VMM intercepts the trap
/// and inspects `func_id` to determine the outcome.
///
/// # Safety
/// Must only be called after `write_mailbox` has flushed the response.
pub unsafe fn hvc_signal(func_id: u64) {
    // SAFETY: HVC is an intentional guest→host exit; x0 is the SMCCC function
    // identifier; x0 is clobbered by the host's return value (ignored here).
    core::arch::asm!(
        "mov x0, {fid}",
        "hvc #0",
        fid = in(reg) func_id,
        out("x0") _,
        options(nostack),
    );
}
