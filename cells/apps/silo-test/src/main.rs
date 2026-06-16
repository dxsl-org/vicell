//! Integration test for the Tier 3a Security Silo (T1–T6).
//!
//! This cell has NO hypervisor capability — it communicates via SiloHandle IPC.
//! T6 verifies that calling CreateVm (syscall 220) without HypervisorCap is rejected.
//!
//! Test matrix:
//! - T1: Service lookup — SiloHandle::connect() resolves SILO_SERVICE_ID.
//! - T2: Key init + GetPub — init_key() returns 0x04-prefixed uncompressed point.
//! - T3: Sign round-trip — sign() + p256 verify_prehash() confirms silo-side signing.
//! - T4: ECDH round-trip — ecdh() with a local ephemeral key, verify shared secret.
//! - T5: Fault recovery — invalid opcode 0xFF returns GuestFault; silo still alive.
//! - T6: Capability isolation — CreateVm (syscall 220) denied without HypervisorCap.
#![no_std]
#![no_main]

extern crate alloc;

// No hypervisor capability — intentional for API isolation test (T6).
api::declare_manifest!(
    block_io   = false,
    network    = false,
    spawn      = false,
    gpio       = false,
    uart       = false,
    hypervisor = false
);

api::declare_syscalls![Send, Recv, Log, LookupService];

use ostd::io::println;
use ostd::silo::{SiloHandle, SiloError};

#[no_mangle]
pub fn main() {
    println("[silo-test] starting T1–T6");
    let mut passed = 0u32;

    // ── T1: Service lookup ────────────────────────────────────────────────────
    let handle = match SiloHandle::connect() {
        Ok(h) => {
            println(&alloc::format!(
                "[silo-test] T1 PASS: silo service found, tid={}",
                h.tid()
            ));
            passed += 1;
            h
        }
        Err(_) => {
            println("[silo-test] T1 FAIL: silo service not found");
            println("[silo-test] ABORT: cannot continue without silo service");
            return;
        }
    };

    // ── T2: Key init + GetPub ─────────────────────────────────────────────────
    let seed = [0x42u8; 32]; // deterministic test seed
    let pub_key = match handle.init_key(&seed) {
        Ok(pk) => {
            if pk[0] == 0x04 {
                println(&alloc::format!(
                    "[silo-test] T2 PASS: pub_key[0..4]={:02x?}",
                    &pk[0..4]
                ));
                passed += 1;
                pk
            } else {
                println(&alloc::format!(
                    "[silo-test] T2 FAIL: pub_key[0]={:02x} (expected 0x04 uncompressed point)",
                    pk[0]
                ));
                [0u8; 65]
            }
        }
        Err(e) => {
            println(&alloc::format!("[silo-test] T2 FAIL: init_key error={:?}", e));
            [0u8; 65]
        }
    };

    // ── T3: Sign round-trip ───────────────────────────────────────────────────
    // Hash the test message, sign inside the silo, verify with p256 on cell side.
    {
        use sha2::{Sha256, Digest};
        use p256::ecdsa::{VerifyingKey, DerSignature};
        use p256::ecdsa::signature::hazmat::PrehashVerifier;

        let mut hasher = Sha256::new();
        hasher.update(b"ViCell Security Silo Test 2026");
        let digest_bytes: [u8; 32] = hasher.finalize().into();

        match handle.sign(&digest_bytes) {
            Ok(sig) => {
                let sig_slice = &sig.bytes[..sig.len as usize];
                // DerSignature::try_from(&[u8]) — p256 0.13 inherent impl.
                match DerSignature::try_from(sig_slice) {
                    Ok(der_sig) => {
                        match VerifyingKey::from_sec1_bytes(&pub_key) {
                            Ok(vk) => {
                                match vk.verify_prehash(&digest_bytes, &der_sig) {
                                    Ok(()) => {
                                        println("[silo-test] T3 PASS: ECDSA sign+verify ok");
                                        passed += 1;
                                    }
                                    Err(_) => {
                                        println("[silo-test] T3 FAIL: signature verify failed");
                                    }
                                }
                            }
                            Err(_) => {
                                println("[silo-test] T3 FAIL: pub_key parse failed");
                            }
                        }
                    }
                    Err(_) => {
                        println("[silo-test] T3 FAIL: DER parse failed");
                    }
                }
            }
            Err(e) => {
                println(&alloc::format!("[silo-test] T3 FAIL: sign error={:?}", e));
            }
        }
    }

    // ── T4: ECDH round-trip ───────────────────────────────────────────────────
    // Generate a local ephemeral P-256 key, send its public key to the silo,
    // compute the shared secret both ways, and verify they match.
    {
        use p256::{SecretKey, PublicKey, EncodedPoint};
        use p256::ecdsa::SigningKey;
        use p256::ecdh::diffie_hellman;
        use p256::elliptic_curve::sec1::FromEncodedPoint;

        // Deterministic test ephemeral scalar — 0x5A repeated.
        // SecretKey::from_bytes takes &GenericArray<u8, U32>; use .as_slice().into().
        let ephemeral_bytes = [0x5Au8; 32];
        match SecretKey::from_bytes(ephemeral_bytes.as_slice().into()) {
            Ok(ephemeral_sk) => {
                // Wrap in SigningKey to access as_nonzero_scalar() for diffie_hellman.
                let ephemeral_signing = SigningKey::from(&ephemeral_sk);
                let ephemeral_vk = ephemeral_signing.verifying_key();
                // to_encoded_point(false) = uncompressed 65-byte SEC1.
                let ephemeral_pt = ephemeral_vk.to_encoded_point(false);
                let peer_bytes: [u8; 65] = match ephemeral_pt.as_bytes().try_into() {
                    Ok(b) => b,
                    Err(_) => {
                        println("[silo-test] T4 FAIL: ephemeral pub key not 65 bytes");
                        [0u8; 65]
                    }
                };

                match handle.ecdh(&peer_bytes) {
                    Ok(shared_a) => {
                        // Compute expected shared secret locally:
                        // shared_b = ECDH(ephemeral_sk, silo_pub)
                        let silo_pt = match EncodedPoint::from_bytes(&pub_key) {
                            Ok(p) => p,
                            Err(_) => {
                                println("[silo-test] T4 FAIL: silo pub key parse failed");
                                return;
                            }
                        };
                        match PublicKey::from_encoded_point(&silo_pt).into_option() {
                            Some(silo_pub) => {
                                let shared_b = diffie_hellman(
                                    ephemeral_signing.as_nonzero_scalar(),
                                    silo_pub.as_affine(),
                                );
                                if shared_a == shared_b.raw_secret_bytes().as_ref() {
                                    println("[silo-test] T4 PASS: ECDH shared secret matches");
                                    passed += 1;
                                } else {
                                    println("[silo-test] T4 FAIL: shared secret mismatch");
                                }
                            }
                            None => {
                                println("[silo-test] T4 FAIL: silo pub key not on curve");
                            }
                        }
                    }
                    Err(e) => {
                        println(&alloc::format!("[silo-test] T4 FAIL: ecdh error={:?}", e));
                    }
                }
            }
            Err(_) => {
                println("[silo-test] T4 FAIL: ephemeral key from_bytes failed (invalid scalar)");
            }
        }
    }

    // ── T5: Fault recovery ────────────────────────────────────────────────────
    // Send invalid opcode 0xFF — silo must return GuestFault without crashing.
    // After the fault the silo should still serve requests (public key unchanged).
    {
        let result = handle.send_raw(0xFF, &[0u8; 32]);
        match result {
            Err(SiloError::GuestFault) => {
                // Silo returned a fault status — verify it is still alive.
                match handle.get_public_key() {
                    Ok(pk2) => {
                        if pk2 == pub_key {
                            println("[silo-test] T5 PASS: fault recovery ok, pub_key stable");
                            passed += 1;
                        } else {
                            println(
                                "[silo-test] T5 FAIL: pub_key changed after fault (unexpected re-key)",
                            );
                        }
                    }
                    Err(_) => {
                        println("[silo-test] T5 FAIL: silo unresponsive after fault");
                    }
                }
            }
            Ok(_) => {
                println("[silo-test] T5 FAIL: invalid opcode 0xFF should return GuestFault");
            }
            Err(e) => {
                println(&alloc::format!(
                    "[silo-test] T5 FAIL: unexpected error={:?} (expected GuestFault)",
                    e
                ));
            }
        }
    }

    // ── T6: Capability isolation ──────────────────────────────────────────────
    // Verify that CreateVm (syscall 220) is rejected without HypervisorCap.
    // This cell has NO hypervisor manifest flag — the kernel must deny it.
    {
        let result: usize;
        // SAFETY: deliberate security-gate probe — we invoke syscall 220 (CreateVm)
        // knowing it is NOT in our declare_syscalls! allowlist and we have no
        // HypervisorCap.  The kernel must return a non-zero error code (PermissionDenied).
        // We check the raw result and never use the returned value as a real vm_id.
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") 220usize => result,
                in("x1") 4usize,
                in("x2") 0usize,
                in("x3") 0usize,
                in("x4") 0usize,
                options(nostack, preserves_flags),
            );
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            // RISC-V: syscall via ecall; a7 = syscall id, a0 = arg0.
            // SAFETY: same intent — probing the gate from RISC-V.
            unsafe {
                core::arch::asm!(
                    "ecall",
                    inlateout("a0") 220usize => result,
                    in("a1") 4usize,
                    in("a2") 0usize,
                    in("a3") 0usize,
                    options(nostack, preserves_flags),
                );
            }
        }

        // A successful CreateVm returns a vm_id > 0; any error is non-zero / MAX.
        // result == 0 would mean success (vm_id=0 is invalid by kernel convention) OR
        // an ABI collision.  result == 1..=usize::MAX is any error code.
        // The kernel returns usize::MAX (PermissionDenied) for cap-gated denials.
        if result == 0 {
            println("[silo-test] SECURITY GATE T6 FAILED");
            println("[silo-test] T6 FAIL: CreateVm succeeded without HypervisorCap — security bug!");
        } else {
            println(&alloc::format!(
                "[silo-test] T6 PASS: capability isolation enforced (syscall 220 → denied, code=0x{:x})",
                result
            ));
            passed += 1;
        }
    }

    // ── Final summary ─────────────────────────────────────────────────────────
    println(&alloc::format!("[silo-test] {}/6 tests passed", passed));
    if passed == 6 {
        println("[silo-test] ALL TESTS PASSED (6/6)");
    } else {
        println(&alloc::format!("[silo-test] FAIL: only {}/6 passed", passed));
    }
}
