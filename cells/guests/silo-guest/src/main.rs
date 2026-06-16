// SPDX-License-Identifier: MPL-2.0
// Silo guest entry point.
//
// This is a bare-metal AArch64 binary that runs inside Stage-2 fenced memory.
// It holds a P-256 private key and exposes signing, ECDH, and public-key
// retrieval to the host via a shared mailbox page at IPA 0x4000_3000.
//
// Event loop contract:
//   1. Guest boots, calls silo_main.
//   2. Executes WFI — host writes request to mailbox, then resumes guest.
//   3. Guest copies mailbox to stack (TOCTOU guard), dispatches, writes response.
//   4. Guest fires HVC to signal host.  Loop back to 2.

#![no_std]
#![no_main]

mod crypto;
mod mailbox;

use core::arch::global_asm;

use crypto::{CryptoResult, SiloState};
use mailbox::{MailboxPage, SiloCmd, HVC_SILO_DONE, HVC_SILO_FAULT, HVC_SILO_READY};

// Include the AArch64 entry stub.  It zeroes BSS and calls `silo_main`.
global_asm!(include_str!("arch/entry.s"));

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Signal the host that something went unrecoverably wrong, then halt.
    // We cannot use the normal dispatch path here (state may be corrupt).
    unsafe {
        mailbox::hvc_signal(HVC_SILO_FAULT);
    }
    loop {
        // SAFETY: WFI is always safe in AArch64 — worst case wakes immediately.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Rust entry point — called from the assembly stub after BSS is zeroed.
///
/// Never returns.  The function is `extern "C"` so the assembly `bl silo_main`
/// finds it with the correct calling convention.
#[no_mangle]
pub extern "C" fn silo_main() -> ! {
    let mut state = SiloState::uninit();

    loop {
        // Wait for the host to deposit work.
        // SAFETY: WFI is always valid in AArch64.
        unsafe { core::arch::asm!("wfi") };

        // Copy the 4 KiB mailbox to the stack before parsing (TOCTOU guard).
        // SAFETY: VMM guarantees the page is valid and the guest has resumed
        // only after the host finished writing.
        let page = unsafe { mailbox::read_mailbox() };

        let (result, hvc_id) = dispatch(&mut state, &page);

        // Build response page — echo seq so host can detect stale responses.
        let mut resp = MailboxPage {
            seq: page.seq,
            cmd: page.cmd,
            resp: result_code(&result),
            _pad: [0; 2],
            data: [0; 4088],
        };
        write_result(&result, &mut resp);

        // Publish response and signal host.
        // SAFETY: we have exclusive write access between WFI wake-up and HVC.
        unsafe {
            mailbox::write_mailbox(&resp);
            mailbox::hvc_signal(hvc_id);
        }
    }
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

fn dispatch(state: &mut SiloState, page: &MailboxPage) -> (CryptoResult, u64) {
    match SiloCmd::from(page.cmd) {
        SiloCmd::Init => {
            // Copy seed to mutable stack buffer so `init` can zero it.
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&page.data[..32]);
            let r = state.init(&mut seed);
            let hvc = if matches!(r, CryptoResult::Ready { .. }) {
                HVC_SILO_READY
            } else {
                HVC_SILO_FAULT
            };
            (r, hvc)
        }
        SiloCmd::Sign => {
            let mut digest = [0u8; 32];
            digest.copy_from_slice(&page.data[..32]);
            let r = state.sign(&digest);
            let hvc = hvc_for(&r);
            (r, hvc)
        }
        SiloCmd::Ecdh => {
            let mut peer = [0u8; 65];
            peer.copy_from_slice(&page.data[..65]);
            let r = state.ecdh(&peer);
            let hvc = hvc_for(&r);
            (r, hvc)
        }
        SiloCmd::GetPub => {
            let r = state.get_pub();
            let hvc = hvc_for(&r);
            (r, hvc)
        }
        SiloCmd::Unknown => (CryptoResult::Fault(0xFF), HVC_SILO_FAULT),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Map a result to the appropriate HVC signal ID.
#[inline]
fn hvc_for(r: &CryptoResult) -> u64 {
    if matches!(r, CryptoResult::Fault(_)) {
        HVC_SILO_FAULT
    } else {
        HVC_SILO_DONE
    }
}

/// Map a result to its response-code byte written into `resp`.
fn result_code(r: &CryptoResult) -> u8 {
    match r {
        CryptoResult::Ready { .. } => 0x00,
        CryptoResult::Signature { .. } => 0x01,
        CryptoResult::SharedSecret(_) => 0x02,
        CryptoResult::PubKey(_) => 0x03,
        CryptoResult::Fault(c) => *c,
    }
}

/// Copy result payload into the response mailbox page.
fn write_result(r: &CryptoResult, page: &mut MailboxPage) {
    match r {
        CryptoResult::Ready { pub_key } => {
            page.data[..65].copy_from_slice(pub_key);
        }
        CryptoResult::Signature { der, len } => {
            let n = *len as usize;
            page.data[0] = *len;
            page.data[1..1 + n].copy_from_slice(&der[..n]);
        }
        CryptoResult::SharedSecret(s) => {
            page.data[..32].copy_from_slice(s);
        }
        CryptoResult::PubKey(k) => {
            page.data[..65].copy_from_slice(k);
        }
        CryptoResult::Fault(c) => {
            page.data[0] = *c;
        }
    }
}
