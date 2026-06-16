#![no_std]
#![no_main]
// unsafe_code is required only in vmm.rs (inline asm for hardware syscall ABI).
// All other modules in this crate avoid unsafe. See vmm.rs for SAFETY docs.

//! Security Silo Service Cell — hosts the bare-metal P-256 key guest.
//!
//! Spawns a minimal VM (32 KB guest RAM), loads the embedded silo-guest binary,
//! seeds the guest key, then serves Sign / Ecdh / GetPub IPC requests.
//!
//! # Guest binary
//! The embedded `silo-guest.bin` at `cells/services/silo/silo-guest.bin` is a
//! placeholder (0 bytes) at compile time.  Build the real binary first:
//!
//!   cargo build --release \
//!     --manifest-path cells/guests/silo-guest/Cargo.toml \
//!     --target aarch64-unknown-none
//!   llvm-objcopy -O binary \
//!     cells/guests/silo-guest/target/aarch64-unknown-none/release/silo-guest \
//!     cells/services/silo/silo-guest.bin
//!
//! Without the real binary the silo will log "guest binary empty" and return
//! without entering the IPC loop — all sign/ecdh requests will timeout.

extern crate alloc;

// Manifest: requires HypervisorCap (allowlist bit 44).
api::declare_manifest!(
    block_io   = false,
    network    = false,
    spawn      = false,
    gpio       = false,
    uart       = false,
    hypervisor = true
);

// Narrow syscall allowlist enforced by the kernel.
api::declare_syscalls![
    Send, Recv, Log,
    CreateVm, CreateVcpu, MapGuestMemory, WriteGuestMemory, RunVcpu, ReadGuestMemory,
];

mod ipc;
mod run_loop;
mod vmm;

use ostd::io::println;

/// Guest RAM: 32 KB (8 pages) — text (8 KB) + bss/stack (4 KB) + mailbox (4 KB) + spare.
const GUEST_RAM_PAGES: usize = 8;
/// Guest IPA base — matches the silo-guest linker script (0x40000000).
const GUEST_IPA_BASE: u64 = 0x4000_0000;
/// Total guest RAM in bytes.
const GUEST_RAM_BYTES: usize = GUEST_RAM_PAGES * 4096;
/// Guest entry PC — first byte of .text at the IPA base.
const GUEST_ENTRY_PC: u64 = GUEST_IPA_BASE;

// PLACEHOLDER: cells/services/silo/silo-guest.bin must be replaced with the
// real stripped binary produced from cells/guests/silo-guest before this cell
// can operate.  See the module doc above for the build command.
static GUEST_BIN: &[u8] = include_bytes!("../silo-guest.bin");

#[no_mangle]
pub fn main() {
    println("[silo] security silo service starting");

    // ── Guard: placeholder binary ─────────────────────────────────────────────
    if GUEST_BIN.is_empty() {
        println("[silo] guest binary empty — build silo-guest.bin first (see module doc)");
        return;
    }
    if GUEST_BIN.len() > GUEST_RAM_BYTES {
        println("[silo] guest binary too large for guest RAM");
        return;
    }

    // ── 1. Create VM ──────────────────────────────────────────────────────────
    let vm_id = vmm::create_vm(GUEST_RAM_PAGES);
    if vm_id == 0 || vm_id == usize::MAX {
        println("[silo] create_vm failed — not at EL2 or OOM");
        return;
    }

    // ── 2. Map guest RAM ──────────────────────────────────────────────────────
    let ret = vmm::map_guest_memory(vm_id, GUEST_IPA_BASE, GUEST_RAM_BYTES, true);
    if ret == usize::MAX {
        println("[silo] map_guest_memory failed");
        return;
    }

    // ── 3. Load flat guest binary ─────────────────────────────────────────────
    let written = vmm::write_guest_memory(vm_id, GUEST_IPA_BASE, GUEST_BIN);
    if written == usize::MAX {
        println("[silo] write_guest_memory failed");
        return;
    }
    println(&alloc::format!("[silo] guest loaded {} bytes", GUEST_BIN.len()));

    // ── 4. Create vCPU ────────────────────────────────────────────────────────
    let vcpu_id = vmm::create_vcpu(vm_id, GUEST_ENTRY_PC);
    if vcpu_id == 0 || vcpu_id == usize::MAX {
        println("[silo] create_vcpu failed");
        return;
    }

    // ── 5. Wait for guest init (HVC_SILO_READY) ───────────────────────────────
    // The guest generates its P-256 key and fires HVC_SILO_READY when done.
    match run_loop::run_until_done(vm_id, vcpu_id) {
        run_loop::SiloRunResult::Done => {
            println("[silo] guest initialised — key ready");
        }
        run_loop::SiloRunResult::Fault(code) => {
            println(&alloc::format!("[silo] guest init fault: 0x{:x}", code));
            return;
        }
        run_loop::SiloRunResult::GuestError => {
            println("[silo] guest init error — aborting");
            return;
        }
    }

    println("[silo] entering IPC loop");

    // ── 6. IPC server loop (never returns under normal operation) ─────────────
    ipc::run(vm_id, vcpu_id);
}
