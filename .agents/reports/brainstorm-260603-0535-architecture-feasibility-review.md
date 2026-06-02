# ViOS Architecture & Feasibility Review

**Date**: 2026-06-03  
**Type**: Brainstorm / Strategic Review  
**Branch**: main  
**Kernel LOC**: ~8,706 | **Total LOC**: ~21,500 | **Crates**: 35

---

## Codebase Snapshot

| Area | LOC | Notes |
|------|-----|-------|
| Kernel | 8,706 | 47 `.rs` files |
| Cells | 5,980 | apps + drivers + services + runtimes |
| HAL | 2,503 | 10 crates, RV64 full, ARM/x86 stubs |
| Libs | 4,284 | types, api, ostd |

**Boot chain (verified)**: OpenSBI → kernel → init → VFS → Config → Shell (`ViOS >`)

---

## Functional Status (as of 2026-06-03)

| Component | Status | Evidence |
|-----------|--------|---------|
| Boot + shell REPL | ✅ Verified | Boot log, interactive input |
| ELF loader + PIE reloc | ✅ Verified | init/shell loaded from disk |
| VFS RamFS + FAT16 | ✅ Verified | mkdir/write/read/unlink |
| VirtIO block + keyboard | ✅ Verified | IRQ ack fix in place |
| Lua 5.4 REPL | ✅ Verified | VA fix applied, multi-line REPL |
| Network (smoltcp) | ⚠️ Partial | DHCP unconfirmed, TCP data path unverified |
| GPU / Compositor | ❌ Broken | setup_framebuffer hangs → opt-in only |
| MicroPython REPL | ⚠️ Partial | Binary links, runtime behavior unverified |
| Hot-migration | ❌ Unexercised | Trait + 5-step design exists, never run |
| ARM64 HAL | ❌ Stub | ~52 LOC |
| x86_64 HAL | ❌ Stub | ~46 LOC |
| Integration tests | ⚠️ Minimal | 2/many pass (QemuRunner harness exists) |

---

## Architecture Assessment

### Strengths

- **SAS + LBI is academically sound** — Theseus, Asterinas, Tock have papers. ViOS is in good company.
- **Nano-kernel < 9K LOC** — auditable, portable, small attack surface.
- **8 Coding Laws** — `#![forbid(unsafe_code)]` in Cells, Vi prefix, no `mod.rs` — discipline with clear rationale.
- **Zero-cost IPC** — direct function call across Cells vs message passing = real perf advantage.
- **Shell + ELF + Lua** — demo-ready stack for real use cases on RV64.

### Risks

1. **"LBI replaces MMU" is only half true.**
   - Type system prevents *accidental* violations.
   - Does NOT prevent violations through `unsafe` code.
   - A compromised Cell has full kernel-space access.
   - **Mitigation**: Acceptable for IoT where all Cells are trusted + signed.

2. **WASM + untrusted code is an unresolved design conflict.**
   - `cells/drivers/wasm/` exists but security model is undefined.
   - Raw SAS + arbitrary WASM bytecode = LBI model collapses.
   - **Decision needed**: trusted-only (keep LBI) vs untrusted (need RISC-V PMP or wasmtime sandbox).

3. **Hot-swap in SAS is extremely hard to get right.**
   - Strong Ref / Weak Ref DAG + Zombie State design is correct direction.
   - Never been exercised → correctness unproven.
   - Formal verification would be needed for production guarantees.

4. **Async + owned buffer = ecosystem friction.**
   - Law 2 (owned buffers, no `&mut [u8]` across async) is necessary but awkward.
   - All external crates (smoltcp, etc.) expect references → bridging layer = bug source.

---

## Use Case Alignment

**User's stated goal**: Edge IoT / embedded (short-term) → specialized full OS for narrow domains (long-term). NOT a general-purpose Linux-alike.

**Verdict**: SAS + LBI is *strongest* for IoT, not weakest.
- IoT = single-vendor, all Cells trusted → Ed25519 signing is maintainable
- Hot-swap = OTA update zero-downtime → killer feature
- SAS = deterministic latency, small footprint, no context-switch → exactly what IoT needs

---

## Scope Assessment

Current roadmap is **over-scoped for apparent team size** (1-2 people):
- 5 architectures (RV64, RV32, ARM64, ARM32, x86_64)
- Compositor + GPU
- WASM runtime
- Network + smoltcp
- Hot-migration
- 2 scripting runtimes (Lua + MicroPython)
- Integration test suite

All of this simultaneously = Phase 1 at 60%, everything partial, nothing production-solid.

---

## Recommended Phasing (Rebalanced)

### Phase 1 — Core IoT Stack (→ Q3 2026)

**Goal**: One real hardware target + verified network + hot-migration working.

| Task | Priority | Rationale |
|------|----------|-----------|
| Verify network: DHCP → TCP loopback | P0 | Blocking IoT story |
| Port to real hardware (VisionFive2 / JH7110) | P0 | QEMU-only ≠ IoT |
| Implement + test hot-migration | P0 | OTA = IoT killer feature |
| Integration tests (7 core-path tests) | P1 | Safety net before scaling |
| Verify MicroPython REPL (1 session) | P2 | Low effort, bonus |
| Mark GPU compositor as `experimental` | P2 | Stop pretending it works |
| Drop AArch32 + RV32 from active dev | — | YAGNI for IoT |

### Phase 2 — Platform Expand (Q4 2026)

| Task | Priority |
|------|----------|
| ARM64 HAL (Raspberry Pi 4/5, Orange Pi 5) | P0 |
| Power management (battery IoT critical) | P1 |
| WASM security model decision + implementation | P1 |

### Phase 3 — Specialized Domains (2027+)

- Domain profiles: robotics, industrial control, firmware AI inference
- x86_64 for development environments
- Security audit + formal modeling for hot-swap

---

## 7 Integration Tests for Core Path

```
1. boot_banner        boot → OpenSBI → "ViOS >"
2. shell_basic        echo, ls, cat, cd
3. vfs_rw             mkdir/write/read/unlink on RamFS
4. network_dhcp       DHCP lease acquired
5. network_tcp        loopback TCP connect/send/recv
6. hotswap_shell      live shell upgrade, history preserved
7. lua_eval           /bin/lua "return 1+1" → 2
```

---

## Critical Open Decision

**WASM security boundary** — must decide before Phase 2:

| Option | Model | Complexity | Fits IoT? |
|--------|-------|-----------|-----------|
| A — Trusted-only Cells | LBI (current) | Low | ✅ Yes |
| B — Untrusted WASM | RISC-V PMP sandbox | High | Depends |
| C — wasmtime isolation | VM-in-SAS | Medium | Conditional |

**Recommendation**: Option A for Phase 1-2. Revisit for Phase 3 if marketplace/plugin story is needed.

---

## Next Actions (for next session)

- [ ] Verify network DHCP: boot QEMU + grep serial for "DHCP lease"
- [ ] Write integration test #4 (network_dhcp) + #5 (network_tcp)
- [ ] Prototype hot-migration exercise: spawn shell → hotswap → verify history
- [ ] Research VisionFive2 / JH7110 boot requirements (U-Boot / OpenSBI version)
- [ ] Mark GPU compositor opt-in in docs + README status table
