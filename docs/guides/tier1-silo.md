# Tier 1 Extended — Hardware Isolation via Silo

> Cryptographic keys and security-sensitive operations in an isolated security domain.

---

## What is a Silo?

A **Silo** is a lightweight hardware-isolated micro-VM running on ARM64 and x86 (not RISC-V). It provides:

- **Exclusive access** to ECDSA/ECDH keys (they never leave)
- **Fault isolation** — a guest fault (invalid instruction) doesn't crash the main kernel
- **Service Cell pattern** — sign/derive operations via IPC

**Platform support**: ARM64 (EL2) and x86 (VMX) only. RISC-V uses Tier 1 Rust (no Silo).

---

## Architecture

```
┌─────────────────┐
│  App Cell       │
│                 │
│ SiloHandle::    │
│  sign()         │  ← RPC to Silo service
│  ecdh()         │
│  init_key()     │
└────────┬────────┘
         │ IPC (Send/Recv)
         ▼
    ┌─────────────┐
    │ Silo Service│
    │ (hypervisor)│  ← Guard hardware isolation
    │             │
    │ P-256 ops   │
    └─────────────┘
```

The Silo runs guest firmware (`silo-guest`) that implements ECDSA/ECDH. Keys are stored in guest memory and never exposed to the main kernel.

---

## SiloHandle API

```rust
use ostd::silo::{SiloHandle, SiloError};

// Connect to the Silo service
let mut handle = SiloHandle::connect()?;
    // → Result<SiloHandle, SiloError>

// Initialize a key from a seed (32 bytes)
let seed = [0x42u8; 32];
let pub_key = handle.init_key(&seed)?;
    // → Result<[u8; 65], SiloError>
    // Returns uncompressed P-256 public key (0x04 prefix)

// Get public key (after init)
let pub_key = handle.get_public_key()?;
    // → Result<[u8; 65], SiloError>

// Sign a message (pre-hashed SHA-256)
use sha2::{Sha256, Digest};
let mut hasher = Sha256::new();
hasher.update(b"my message");
let digest: [u8; 32] = hasher.finalize().into();

let sig = handle.sign(&digest)?;
    // → Result<SiloSignature, SiloError>
    // sig.bytes[..sig.len] is DER-encoded signature

// ECDH with peer public key (65 bytes, uncompressed)
let peer_pub = [0x04u8; 65];  // peer's public key
let shared_secret = handle.ecdh(&peer_pub)?;
    // → Result<[u8; 32], SiloError>

// Send raw command (advanced)
let result = handle.send_raw(opcode, &arg_data)?;
    // → Result<Vec<u8>, SiloError>
```

---

## Error Handling

```rust
use ostd::silo::SiloError;

match handle.init_key(&seed) {
    Ok(pk) => { /* use pk */ }
    Err(SiloError::ServiceNotFound) => {
        println("Silo not available (RISC-V / single-cell boot)");
    }
    Err(SiloError::GuestFault) => {
        println("Invalid guest operation — key may be corrupted");
    }
    Err(SiloError::Timeout) => {
        println("Silo IPC timeout");
    }
    Err(SiloError::InvalidResponse) => {
        println("Silo returned unexpected format");
    }
}
```

---

## Manifest & Syscalls

Silo access requires no manifest flag (it's a service). Declare:

```rust
api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, Exit, LookupService];
```

The Silo service is discovered via `sys_lookup_service()` (wrapped by `SiloHandle::connect()`).

---

## Example: Sign and Verify

```rust
extern crate alloc;
use sha2::{Sha256, Digest};
use p256::ecdsa::{VerifyingKey, DerSignature};
use p256::ecdsa::signature::hazmat::PrehashVerifier;

fn sign_and_verify() -> Result<(), Box<dyn core::fmt::Debug>> {
    let mut handle = ostd::silo::SiloHandle::connect()?;
    
    // Initialize with a deterministic seed
    let seed = [0x99u8; 32];
    let pub_key = handle.init_key(&seed)?;
    
    // Create message digest
    let mut hasher = Sha256::new();
    hasher.update(b"Important transaction");
    let digest: [u8; 32] = hasher.finalize().into();
    
    // Sign in Silo
    let sig = handle.sign(&digest)?;
    
    // Verify locally using p256 crate
    let sig_slice = &sig.bytes[..sig.len as usize];
    let der_sig = DerSignature::try_from(sig_slice)?;
    let vk = VerifyingKey::from_sec1_bytes(&pub_key)?;
    vk.verify_prehash(&digest, &der_sig)?;
    
    println("Signature verified!");
    Ok(())
}
```

---

## Example: Key Derivation via ECDH

```rust
use p256::{SecretKey, EncodedPoint};

fn ecdh_derive() -> Result<(), Box<dyn core::fmt::Debug>> {
    let mut handle = ostd::silo::SiloHandle::connect()?;
    
    // Server (Silo) generates long-term key
    let server_seed = [0xAAu8; 32];
    let server_pub = handle.init_key(&server_seed)?;
    
    // Client generates ephemeral key
    let client_secret = SecretKey::random(&mut rand::thread_rng());
    let client_pub_point = client_secret.public_key().to_encoded_point(false);
    let client_pub_bytes: [u8; 65] = client_pub_point
        .as_bytes()
        .try_into()?;
    
    // Server (in Silo) performs ECDH
    let shared_secret = handle.ecdh(&client_pub_bytes)?;
    // shared_secret is now a 32-byte session key
    
    println("Derived {} bytes", shared_secret.len());
    Ok(())
}
```

---

## Platform-Specific Notes

### ARM64 (EL2 hypervisor)

- Requires ARM64 board with virtualization extensions (EL2)
- QEMU ARM64 supports it; real boards (RK3588, etc.) have EL2
- Silo firmware boots in secure isolated EL2 context

### x86 (VMX hypervisor)

- Requires x86 CPU with VMX (Intel) or SVM (AMD)
- QEMU x86 with `-enable-kvm` or TCG simulation
- Silo firmware boots as VM with isolated memory

### RISC-V

- **Not supported**. No Silo on RISC-V.
- H-ext (hypervisor extension) too new; adoption uncertain.
- For cryptographic keys, use Tier 1 Rust + ostd's p256 crate locally (keys stay in Rust cell memory).

---

## Security Model

1. **Keys never leave the Silo**: ECDSA/ECDH operations happen in guest; only derived data (public key, signature, shared secret) crosses the boundary.
2. **Fault isolation**: if guest code faults (invalid opcode), it returns `SiloError::GuestFault` without crashing the kernel.
3. **IPC authenticity**: the calling Cell's TID is validated by the kernel; only authenticated requests are forwarded.

---

## When to Use Tier 1 Extended (Silo)

✅ Storing private keys (long-term or session)  
✅ ECDSA signing (blockchain, auth, etc.)  
✅ ECDH key exchange (TLS, protocols)  
✅ Fault isolation (untrusted guest code)  

❌ General-purpose apps → use Tier 1 Rust + SDK L1  
❌ Network I/O in Silo → not supported (isolated)  
❌ RISC-V → use Tier 1 Rust locally  

---

## Canonical Example

See [cells/apps/silo-test/src/main.rs](../../cells/apps/silo-test/src/main.rs) — comprehensive test suite covering:
- T1: Service discovery
- T2: Key init + public key export
- T3: Sign and verify (p256 crate)
- T4: ECDH with ephemeral key
- T5: Fault recovery
- T6: Capability isolation

---

## Build & Run (ARM64 QEMU)

```bash
# QEMU ARM64 with Silo support (auto-enabled)
./run-arm64.ps1

# On shell:
silo-test
```

Output:
```
[silo-test] starting T1–T6
[silo-test] T1 PASS: silo service found, tid=...
[silo-test] T2 PASS: pub_key[0..4]=04 ...
...
[silo-test] ALL TESTS PASSED (6/6)
```

---

## Troubleshooting

**SiloError::ServiceNotFound?**  
→ Silo not available: running on RISC-V, single-cell boot, or VM not launched. Use Tier 1 Rust locally.

**GuestFault on init_key?**  
→ Seed data corrupted or invalid. Try a different seed.

**Signature verification fails?**  
→ Check that the digest is exactly 32 bytes (SHA-256) and DER-decoded correctly.

---

## Next Steps

- See [system-architecture.md](../system-architecture.md) § Tier 3 (hypervisor) for internal Silo design.
- For TLS, see [project-tls-plan.md](../project-tls-plan.md).
- For cryptographic libraries: `p256`, `sha2`, `aes-gcm` all work in Rust Cells.
