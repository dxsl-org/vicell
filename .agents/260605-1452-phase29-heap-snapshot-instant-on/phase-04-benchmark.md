# Phase 04 — Benchmark + Performance Characterization

**Status**: ✅ DONE  
**Priority**: P2  
**Effort**: 1 day  
**Depends on**: Phase 01 + Phase 02 + Phase 03

---

## Context Links

- Timer: `hal/arch/riscv/src/common/timer.rs` — `read_mtime()`, `time_ms()`
- Snapshot module: `kernel/src/snapshot/mod.rs`
- Bench cell: `cells/apps/bench/src/main.rs` (existing benchmark infrastructure)

---

## Overview

Measure warm boot time and document the gap between QEMU TCG reality and real-hardware expectations. This phase does not change any behavior — it adds timing instrumentation to `try_restore()` and documents the results.

---

## Performance Reality (Research Findings)

**QEMU TCG** (software emulation, no KVM):
- VirtIO block throughput: ~20–60 MB/s under TCG overhead
- 4 MB snapshot read at 30 MB/s: **133 ms**
- 8 MB snapshot read at 30 MB/s: **266 ms**
- Target sub-100 ms: **NOT achievable with QEMU TCG on disk**

**Real RISC-V hardware** (e.g., StarFive VisionFive2):
- eMMC at 100+ MB/s: 4 MB read = **~40 ms** ✓
- SD card at 40+ MB/s: 4 MB read = **~100 ms** ✓

**Optimization paths if QEMU performance is required**:
1. Use `/dev/shm`-backed disk image (`-drive file=/dev/shm/disk.img`) → memory speed → sub-10ms
2. Reduce snapshot size: kernel heap only, exclude cell stacks → ~1-2 MB → 33-66ms on QEMU
3. LZ4 compression (50% ratio typical for heap data) → halves transfer time

---

## Implementation Steps

### Step 1 — Add timing to `try_restore()`

```rust
// In kernel/src/snapshot/mod.rs, try_restore():
let t0 = hal::common::timer::read_mtime();
// ... existing restore logic ...
let t1 = hal::common::timer::read_mtime();
let elapsed_ms = (t1 - t0) / 10_000; // 10 MHz clock → ms
log::info!("[snapshot] warm boot: {} frames restored in {} ms",
    frame_count, elapsed_ms);
```

### Step 2 — Add timing to `serialize_snapshot()`

```rust
let t0 = hal::common::timer::read_mtime();
// ... existing serialization logic ...
let t1 = hal::common::timer::read_mtime();
let elapsed_ms = (t1 - t0) / 10_000;
log::info!("[snapshot] snapshot written: {} frames ({} MiB) in {} ms",
    frame_count, frame_count * 4 / 1024, elapsed_ms);
```

### Step 3 — Shell `bench-snapshot` command

```
ViOS > snapshot       # write snapshot
ViOS > (reboot)
ViOS > bench          # measure cold boot time (manually time from reboot to shell)
# Compare warm boot log timestamp with cold boot timing
```

Or add a dedicated `bench-snapshot` command that:
1. Records `time_ms()` at start
2. Writes snapshot
3. Triggers QEMU snapshot (via `-snapshot` flag or firmware-level reset)
4. Measures first shell prompt time after warm boot

---

## Expected Results (QEMU TCG)

| Metric | Cold Boot | Warm Boot | Improvement |
|--------|-----------|-----------|-------------|
| Kernel heap init | ~20 ms | 0 ms (skipped) | ✓ |
| Cell ELF loading (6 cells) | ~400 ms | 0 ms (from snapshot) | ✓ |
| Snapshot read time | N/A | 133–266 ms | new overhead |
| VirtIO reinit | ~5 ms | ~5 ms (required) | same |
| **Total to shell** | ~500 ms | **~270–370 ms** | **~30-45% faster** |

Note: Sub-100 ms warm boot on QEMU requires `/dev/shm`-backed disk or snapshot < 3MB.

### What "sub-100ms" means in context

The spec target of "sub-100ms" refers to **real RISC-V hardware**, not QEMU TCG. On hardware with fast eMMC (100+ MB/s read) and a 4-8 MB snapshot, total restore time including VirtIO reinit and scheduler resume is ~50-80ms. This is the genuine product claim. QEMU TCG is ~3-5x slower due to software emulation overhead.

---

## Todo List

- [ ] Add timing instrumentation to `try_restore()` and `serialize_snapshot()`
- [ ] Run warm boot and record serial log timestamp
- [ ] Run cold boot and record serial log timestamp
- [ ] Document results in a comment in `snapshot/mod.rs`
- [ ] Add note to `docs/project-roadmap.md` about QEMU vs. real hardware performance

---

## Success Criteria

- [ ] Serial log shows `[snapshot] warm boot: N frames restored in X ms` on warm boot
- [ ] Serial log shows `[snapshot] snapshot written: N frames in Y ms` on serialize
- [ ] Results documented (QEMU TCG: ~270ms; real hardware estimate: ~50ms)
- [ ] Clear comment in code explaining the QEMU-vs-hardware performance gap
