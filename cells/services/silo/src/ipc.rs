//! IPC server: receive Sign / Ecdh / GetPub requests; relay through mailbox; respond.
//!
//! Wire layout (no unsafe ptr casts — all fields decoded manually):
//!
//! SiloRequest (128 bytes):
//!   [0]      opcode : u8
//!   [1..31]  _pad   : [u8; 31]
//!   [32..127] data  : [u8; 96]
//!
//! MailboxPage (4096 bytes):
//!   [0..3]   seq    : u32 LE
//!   [4]      cmd    : u8
//!   [5]      resp   : u8
//!   [6..7]   _pad   : [u8; 2]
//!   [8..4095] data  : [u8; 4088]
//!
//! SiloResponse (128 bytes):
//!   [0]      status : u8
//!   [1]      len    : u8
//!   [2..3]   _pad   : [u8; 2]
//!   [4..127] data   : [u8; 124]

extern crate alloc;

use ostd::io::println;
use ostd::syscall::{sys_recv, sys_send, SyscallResult};
use types::silo::{SiloRespCode, MAILBOX_IPA};
use crate::{run_loop, vmm};

// ── Mailbox field offsets (MailboxPage = 4096 bytes) ─────────────────────────
const MB_OFF_SEQ: usize = 0;   // u32 LE
const MB_OFF_CMD: usize = 4;   // u8
const MB_OFF_RESP: usize = 5;  // u8
const MB_OFF_DATA: usize = 8;  // [u8; 4088]

// ── SiloRequest field offsets (128 bytes) ─────────────────────────────────────
const REQ_OFF_OPCODE: usize = 0;   // u8
const REQ_OFF_DATA: usize = 32;    // [u8; 96]
const REQ_DATA_LEN: usize = 96;

// ── SiloResponse field offsets (128 bytes) ────────────────────────────────────
const RESP_OFF_STATUS: usize = 0;  // u8
const RESP_OFF_LEN: usize = 1;     // u8
const RESP_OFF_DATA: usize = 4;    // [u8; 124]
const RESP_DATA_LEN: usize = 124;

const IPC_MSG_SIZE: usize = 4096;
const MAILBOX_SIZE: usize = 4096;

/// Main IPC service loop. Never returns.
///
/// Precondition: `vm_id` and `vcpu_id` are valid, guest RAM is mapped, guest
/// binary is loaded, and the guest has already received its initial SILO_READY
/// HVC (caller handles the init run before entering this loop if needed).
pub fn run(vm_id: usize, vcpu_id: usize) -> ! {
    let mut buf = [0u8; IPC_MSG_SIZE];
    let mut seq: u32 = 0;

    loop {
        // Wait for a request from any sender (mask = 0 = accept all).
        let sender = match sys_recv(0, &mut buf) {
            SyscallResult::Ok(tid) if tid > 0 => tid,
            _ => continue,
        };

        // Decode SiloRequest fields from raw buffer (no unsafe).
        let opcode = buf[REQ_OFF_OPCODE];
        let mut req_data = [0u8; REQ_DATA_LEN];
        req_data.copy_from_slice(&buf[REQ_OFF_DATA..REQ_OFF_DATA + REQ_DATA_LEN]);

        // Build the 4 KiB mailbox page manually.
        let mut mb = [0u8; MAILBOX_SIZE];
        mb[MB_OFF_SEQ..MB_OFF_SEQ + 4].copy_from_slice(&seq.to_le_bytes());
        mb[MB_OFF_CMD] = opcode;
        mb[MB_OFF_RESP] = SiloRespCode::Fault as u8; // guest overwrites on success
        // Copy request payload into mailbox data section (first 96 bytes).
        mb[MB_OFF_DATA..MB_OFF_DATA + REQ_DATA_LEN].copy_from_slice(&req_data);
        seq = seq.wrapping_add(1);

        // Write mailbox into guest RAM.
        let written = vmm::write_guest_memory(vm_id, MAILBOX_IPA, &mb);
        if written == usize::MAX {
            println("[silo] write_guest_memory failed — dropping request");
            send_error_response(sender, 0xFD);
            continue;
        }

        // Resume the guest; it reads the mailbox, processes the request, and
        // writes the response + fires HVC_SILO_DONE (or HVC_SILO_FAULT).
        let run_result = run_loop::run_until_done(vm_id, vcpu_id);

        // Read back the mailbox to extract the guest's response.
        let mut resp_mb = [0u8; MAILBOX_SIZE];
        let read = vmm::read_guest_memory(vm_id, MAILBOX_IPA, &mut resp_mb);
        if read == usize::MAX {
            println("[silo] read_guest_memory failed");
            send_error_response(sender, 0xFC);
            continue;
        }

        // Encode SiloResponse into a 128-byte buffer (no unsafe).
        let mut out = [0u8; 128];
        match run_result {
            run_loop::SiloRunResult::Done => {
                let resp_code = resp_mb[MB_OFF_RESP];
                out[RESP_OFF_STATUS] = 0;
                build_response_data(&resp_mb[MB_OFF_DATA..], resp_code, &mut out);
            }
            run_loop::SiloRunResult::Fault(code) => {
                out[RESP_OFF_STATUS] = 0xFF;
                out[RESP_OFF_DATA] = code;
            }
            run_loop::SiloRunResult::GuestError => {
                out[RESP_OFF_STATUS] = 0xFF;
                out[RESP_OFF_DATA] = 0xFE; // internal guest error sentinel
            }
        }

        let _ = sys_send(sender, &out);
    }
}

/// Fill the response payload section of `out` based on `resp_code`.
///
/// Mailbox data layout per response code:
///   Signature (1): data[0] = DER length, data[1..=len] = DER bytes (max 72 B)
///   Secret    (2): data[0..32] = raw shared secret
///   Ready     (0) / PubKey (3): data[0..65] = uncompressed P-256 public key
fn build_response_data(mb_data: &[u8], resp_code: u8, out: &mut [u8; 128]) {
    if resp_code == SiloRespCode::Signature as u8 {
        let der_len = mb_data[0] as usize;
        let copy_len = der_len.min(RESP_DATA_LEN - 1); // leave [0] for len field
        out[RESP_OFF_LEN] = copy_len as u8;
        out[RESP_OFF_DATA..RESP_OFF_DATA + copy_len]
            .copy_from_slice(&mb_data[1..1 + copy_len]);
    } else if resp_code == SiloRespCode::Secret as u8 {
        out[RESP_OFF_LEN] = 32;
        out[RESP_OFF_DATA..RESP_OFF_DATA + 32].copy_from_slice(&mb_data[..32]);
    } else if resp_code == SiloRespCode::Ready as u8
        || resp_code == SiloRespCode::PubKey as u8
    {
        out[RESP_OFF_LEN] = 65;
        out[RESP_OFF_DATA..RESP_OFF_DATA + 65].copy_from_slice(&mb_data[..65]);
    } else {
        // Unexpected response code — signal fault.
        out[RESP_OFF_STATUS] = 0xFF;
        out[RESP_OFF_DATA] = 0xFB; // unknown resp_code sentinel
    }
}

/// Send a minimal error response back to `sender` with the given error byte.
fn send_error_response(sender: usize, code: u8) {
    let mut out = [0u8; 128];
    out[RESP_OFF_STATUS] = 0xFF;
    out[RESP_OFF_DATA] = code;
    let _ = sys_send(sender, &out);
}
