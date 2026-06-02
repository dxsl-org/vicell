# ViOS SAS Architecture — Deep Risk Analysis

**Date**: 2026-06-03  
**Scope**: Single Address Space security model — design vs. implementation gap  
**Evidence base**: Direct code reading (not spec)

---

## Key Evidence (from code)

### Privilege Model (verified correct)

```rust
// task.rs:216,244 — ALL Cells spawn with:
task.trap_frame.sstatus = 0x6020; // SPP=0 = U-mode (Ring 3/User)

// Kernel context:
task.context.sstatus = 0x42120;   // SPP=1 = S-mode (Ring 0/Supervisor)
```

**Kernel = S-mode. Cells = U-mode. Hardware correctly isolates kernel from Cells.**  
Note: CLAUDE.md says "all runs in Ring 0" — this is misleading. Cells are U-mode.

### No SATP Switching (verified: shared address space)

```rust
// hal/arch/riscv/src/rv64/context.rs — Context struct has NO satp field.
// Context switch does NOT change page table.
// All tasks share KERNEL_ROOT — one global page table.
```

All Cells share one page table. Cell A running in U-mode can dereference Cell B's VA directly — no hardware barrier between Cells.

### No Signature Verification (verified: not implemented)

```rust
// loader.rs:44-69 — spawn_from_path:
pub fn spawn_from_path(path: &str) -> ViResult<usize> {
    let elf_bytes = early::EarlyLoader::read_file(path)?;
    // No Ed25519 check. No trusted manifest. No allowlist.
    crate::task::spawn_from_mem(&elf_bytes, ...)
}
```

Ed25519 signature chain exists only in docs/01-core.md spec. Zero implementation.

### SpawnFromPath — Any Cell Can Spawn Any Cell

```rust
// syscall.rs:755-775:
Syscall::SpawnFromPath { path_ptr, path_len } => {
    // No caller_id authorization check.
    // No privilege requirement.
    // Any U-mode Cell can invoke this.
    crate::loader::spawn_from_path(path_str)
}
```

### SUM=1 + No Ownership Check in validate_user_buf

```rust
// main.rs:213: SUM=1 set PERMANENTLY — kernel can access all U-mode pages.
// validate_user_buf checks: NULL, overflow, max_len.
// Does NOT check: does ptr belong to the calling Cell?
// → Cell A can supply Cell B's VA → kernel reads/writes Cell B's data.
```

---

## Threat Model Assessment

### "Trusted Cell" Model (all Cells signed by ViOS Lab)

| Defense | Status | Evidence |
|---------|--------|---------|
| Kernel ↔ Cell hardware isolation | ✅ Working | U-bit enforced by MMU |
| Cell ↔ Cell hardware isolation | ❌ Missing | Shared SATP, no per-Cell page table |
| Signing pipeline enforcement | ❌ Not implemented | `spawn_from_path` has no sig check |
| Spawn authorization | ❌ Missing | Any Cell can call `SpawnFromPath` |
| Syscall ptr ownership check | ❌ Missing | `validate_user_buf` no ownership |
| SHM handle ACL | ❌ Missing | Global pool, enumerable (self-documented) |
| Hotswap CellId transfer | ❌ Not implemented | Step 5 comment: "routes naturally" |

### When does the threat model hold?

✅ **Safe assumption**: all Cells are trusted, signed, no adversarial code.  
→ LBI (Rust type system) prevents accidental aliasing between Cells.  
→ Hardware (U-bit) prevents Cells from corrupting kernel.  
→ System is stable and functional for IoT trusted-vendor model.

❌ **Unsafe assumption**: any of the following —  
- Third-party apps (user-installed Cells)
- WASM with untrusted bytecode
- Cells from multiple publishers
- Any path where an attacker controls a file on disk

---

## Bypass Vulnerabilities (in priority order)

### BV-1: No Signing Gate = Trust by Convention Only (Critical)

Any ELF on disk loads as a Cell. An attacker with disk write access (via a compromised Cell with VFS write cap, or physical access) can spawn arbitrary Ring-3 code in the SAS.

**Fix**: Implement `verify_cell_signature(elf_bytes)` in `spawn_from_path` before `spawn_from_mem`.  
Requires: Ed25519 pubkey embedded in kernel, `.vios_sig` section in signed ELFs.

### BV-2: Inter-Cell Memory Visibility (Structural)

No per-Cell SATP switch. All U-mode pages are visible to all U-mode tasks.

```
Cell A at VA 0x00400000 can read/write:
  Cell B at VA 0x00800000  ← no hardware barrier
  Cell C at VA 0x02000000  ← no hardware barrier
```

**Fix cho Tier 1 (Trusted Cells)**: Accept + document — LBI handles accidental access, signing handles trust gate. Per-Cell SATP không cần thiết.  
**Fix cho Tier 2 (Untrusted Cells)**: WASM sandbox — bytecode không có raw pointer, validator + runtime bounds-check đảm bảo isolation mà không cần SATP switch.  
**Fix cho Tier 3 (Legacy/Extreme)**: Hypervisor + Stage-2 paging — hardware isolation hoàn toàn.

### BV-3: SpawnFromPath Unrestricted (High)

Any Cell can spawn any Cell from disk. Shell exploit → spawn `/bin/attacker`.

**Fix**: Require `SpawnCap` capability to invoke `SpawnFromPath`. Only `init` holds `SpawnCap` at boot.

### BV-4: SUM=1 + Shared Page Table = Cross-Cell Kernel Read (Medium)

`SUM=1` is permanent. `validate_user_buf` doesn't verify ownership.  
Cell A supplies ptr = Cell B's stack → `Syscall::Read` copies Cell B's data to Cell A's buffer.

**Fix**: Track per-task VA ranges in CellRegistry; validate ptr is within caller's allocated range.

### BV-5: SHM Handle Enumeration (Medium, self-documented)

```rust
// syscall.rs: "any cell that knows a peer's outstanding handle can map it"
```

**Fix**: Add per-owner ACL to `SHM_HANDLES`.

### BV-6: Hotswap DoS via Deserialize Failure (Latent)

If new Cell's deserialize fails, old Cell stays frozen permanently. No rollback.

**Fix**: Keep old Cell in Frozen-Rollback state; unfreeze on deserialization failure.

---

## What the Design Got Right

1. **S-mode vs U-mode split** is correct and implemented. Kernel is hardware-protected from Cells.
2. **3-Tier isolation model** is the right paradigm — LBI → WASM sandbox → Hypervisor covers all threat levels without per-Cell SATP.
3. **WASM Tier 2 > per-Cell SATP** for untrusted apps: WASM validator enforces code semantics safety (no raw pointers expressible in bytecode), not just memory access barriers.
4. **Tier 3 Hypervisor (Stage-2 paging)** is the correct solution for Spectre-class side channels — acknowledged as known limitation of SAS, not swept under the rug.
5. **Capability table** design (CapId, grant_depth, lease expiry) is sound.  
6. **VA boundary check** in ELF loader prevents Cell ELF from mapping kernel space.
7. **`#![forbid(unsafe_code)]`** in pure Cells is enforced at compile time.
8. **Owned buffer IPC** (Law 2) prevents lifetime violations across async boundaries.

---

## Fix Priority

### Phase 1 — IoT Trusted-Cell Model (current scope)

| Priority | Fix | Effort |
|----------|-----|--------|
| P0 | Implement Ed25519 signature gate in `spawn_from_path` | Medium |
| P0 | Add `SpawnCap` capability — only `init` can spawn Cells | Low |
| P1 | Track per-Cell VA ranges in CellRegistry for ptr validation | Medium |
| P1 | Hotswap rollback on deserialize failure | Low |
| P2 | Per-owner ACL on SHM handles | Low |

### Phase 2 — Full OS với untrusted 3rd-party Cells

| Priority | Fix | Effort |
|----------|-----|--------|
| P0 | Implement WASM runtime (`cells/drivers/wasm/`) + validator | High |
| P0 | WASM linear memory allocator + bounds-check integration | High |
| P1 | WASI interface cho Tier 2 Cells (filesystem, network, IPC) | Medium |
| P2 | Hypervisor Cell + Stage-2 paging cho Tier 3 (legacy Linux) | Very High |

**Không cần per-Cell SATP** — Tier 2 WASM sandbox giải quyết inter-Cell isolation cho untrusted Cells mà không cần page table switch.

---

## Summary

The SAS architecture design is internally consistent for a trusted-Cell IoT model.  
The hardware privilege model (S-mode kernel / U-mode Cells) is correctly implemented.  
The critical failure is that the **trust enforcement layer** (signing, spawn authorization, ptr ownership) exists only in the spec — not a single line of code implements it.  

Without the signing gate: the hardware privilege model protects the **kernel** from Cells, but does NOT protect **Cells from each other** (shared SATP) or prevent **untrusted code from being spawned** (no sig check).

---

## Conclusion: Nếu implement đúng thiết kế thì vấn đề có được giải quyết?

**Câu trả lời ngắn: Đúng — thiết kế 3-tier đã giải quyết hoàn toàn, kể cả untrusted 3rd-party Cells.**

### Thiết kế 3-Tier đã có trong spec (docs/05-application.md)

```
┌──────────────────────────────────────────────────────────────────┐
│  Tier 1: Native Rust         │  LBI (Compiler)    │  Trusted     │
│  Tier 2: WASM                │  Software Sandbox  │  Untrusted   │
│  Tier 3: Hypervisor Cell     │  Stage-2 Paging    │  Legacy/     │
│                              │  (Hardware)        │  Ultra-sens  │
└──────────────────────────────────────────────────────────────────┘
```

**Tier 2 WASM là lời giải cho untrusted 3rd-party Cells.**

WASM bytecode không có khái niệm raw pointer — chỉ có linear memory indices. WASM Validator kiểm tra toàn bộ memory access pattern tại load time. Runtime bounds-check mọi access. Cell độc hại viết bằng WASM **không thể diễn đạt** `*(other_cell_addr)` trong bytecode — sandbox nằm trong semantics của ngôn ngữ, không phải hardware page table.

Đây còn mạnh hơn per-Cell SATP:
- SATP chặn cross-cell memory access nhưng không chặn ROP / arbitrary code execution
- WASM validator đảm bảo code semantics an toàn từ khi load — không cần hardware barrier

**Tier 3 Hypervisor với Stage-2 paging** giải quyết nốt:
- Legacy Linux/Windows/Android apps chưa port sang ViOS
- Sensitive silos: private key operations, data cực nhạy
- Spectre/Meltdown side-channels (known limitation của SAS model, được Tier 3 cô lập hoàn toàn)

**Phân tích trước của tôi ("cần per-Cell SATP cho untrusted Cells") là sai** — thiết kế giải quyết điều này qua WASM sandbox, không cần per-Cell page table.

---

### Các vấn đề được giải quyết hoàn toàn (khi implement đúng spec)

| Vấn đề | Giải pháp thiết kế | Kết quả |
|--------|-------------------|---------|
| Không có sig gate | Ed25519 trong `spawn_from_path` | ✅ Resolved — attacker với disk access vẫn không spawn được |
| SpawnFromPath unrestricted | `SpawnCap` — chỉ `init` có quyền | ✅ Resolved — privilege escalation path bị chặn |
| validate_user_buf no ownership | Track per-Cell VA range, check caller | ✅ Resolved — cross-Cell syscall read không còn |
| SHM handle enumeration | Per-owner ACL trên SHM_HANDLES | ✅ Resolved |
| Hotswap: no CellId transfer | Implement routing table update ở Step 5 | ✅ Resolved |
| Hotswap: 64KB hard cap | Dynamic Vec / chunked serialization | ✅ Resolved |
| Hotswap: no rollback | Frozen-Rollback state, unfreeze on fail | ✅ Resolved |

### Vì sao pure Rust Cells được giải quyết hoàn toàn

Đây là điểm cốt lõi của thiết kế LBI:

```
Cell A — #![forbid(unsafe_code)]
  → Rust type system: không thể tạo *const T trỏ vào VA tùy ý
  → Không thể dereference memory ngoài phạm vi owned/borrowed
  → Kể cả shared SATP: safe Rust không có cú pháp để đọc Cell B's memory
```

Với pure Rust Cells: **hardware inter-Cell SATP isolation không cần thiết**.  
Rust type system là "software MMU" — mạnh hơn hardware ở điểm không thể bị bypass bởi compiler-safe code.

### Vấn đề còn lại: C-runtime Cells (Lua, MicroPython)

Đây là **ngoại lệ duy nhất** trong thiết kế — và là giới hạn đã biết của mọi Language-Based Isolation system:

```
Lua Cell (cells/runtimes/lua/) — dùng unsafe FFI để bind Lua C API
MicroPython Cell — tương tự

C code bên trong CÓ THỂ:
  memcpy(dst, cell_b_stack_addr, size)  → đọc Cell B's data
  *(char*)cell_b_heap_ptr = 0           → ghi vào Cell B's heap
```

**Tuy nhiên**, đây không phải lỗ hổng kiến trúc mà là **trade-off đã được chấp nhận**, vì:

1. **Lua/MicroPython interpreter là trusted code** — được ký bởi ViOS Lab, cùng trust level với kernel's own `unsafe` blocks.
2. **Scripts chạy bên trong VM đều bị sandbox** — Lua script người dùng viết không có quyền gọi C pointer arithmetic; Lua VM kiểm soát điều này.
3. **Tương đương với kernel unsafe** — kernel cũng có nhiều `unsafe` blocks; nếu kernel bị exploit thì toàn hệ thống hỏng. C-runtime Cell cũng vậy — khi đã trusted + signed + audited thì risk là implementation bugs, không phải architectural holes.

**Mitigation đủ dùng cho IoT**: Treat Lua/MicroPython interpreter như kernel component — audit kỹ, pin version, không nhận update không được ký.

### Verdict cuối cùng

```
                         HIỆN TẠI (v0.2)       SAU KHI IMPLEMENT ĐẦY ĐỦ
                         ──────────────────     ────────────────────────────────
Kernel isolation         ✅ Working             ✅ Working (S-mode hardware)
Trusted Rust Cells       ❌ No trust gate       ✅ Resolved — signing + SpawnCap
C-runtime Cells          ❌ No trust gate       ✅ Resolved — signing + audit
                                                ⚠️  C bug risk = kernel-level risk
                                                    (known, managed by design)
Untrusted 3rd-party      ❌ Not implemented     ✅ Resolved — WASM Tier 2 sandbox
Legacy Linux/Windows     ❌ Not implemented     ✅ Resolved — Tier 3 Hypervisor
Spectre side-channel     ❌ Not mitigated       ✅ Resolved — Tier 3 VM isolation
```

**Kiến trúc SAS + LBI + 3-Tier của ViOS là sound và complete về mặt thiết kế.**

Mọi threat category đều có giải pháp trong spec:
- Implementation gaps (v0.2) là **engineering debt**, không phải architectural flaws.
- Untrusted Cells được giải quyết bằng **WASM sandbox**, không cần per-Cell SATP — thực ra là lời giải mạnh hơn.
- Spectre/side-channel được giải quyết bằng **Tier 3 Hypervisor**, không phải bị bỏ qua.

Nhất quán với tiền lệ học thuật: Theseus OS (UC Santa Cruz) dùng cùng SAS + Rust LBI approach. WASM sandbox cho untrusted code là industry-proven (WebAssembly trong browsers cùng cơ chế).
