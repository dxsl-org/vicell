# Cellos Performance Baseline Report

> **Status:** Initial baseline — QEMU measurements pending first CI run.
> Updated weekly by `.github/workflows/perf.yml`.

---

## PDR Targets (v1.0 Requirements)

| Metric | Target | Margin | Notes |
|--------|--------|--------|-------|
| Context-switch latency | < 100 µs | ≥ 2× in QEMU | Measured via double `sys_yield` round-trip |
| IPC send/recv round-trip | < 50 µs | ≥ 2× in QEMU | 64-byte message to VFS cell and back |
| Syscall overhead (`Yield`) | < 10 µs | ≥ 2× in QEMU | Single ecall → return to U-mode |
| Kernel + 3 services footprint | < 10 MB | — | Init + Config + VFS + Shell combined |

QEMU measurements show *relative* trends well but undercount due to JIT translation
overhead.  All targets must be met with a 2× safety margin to account for this.

---

## Methodology

### Timer Resolution

The bench cell reads ticks via `sys_get_time()` → kernel `GetTime` syscall →
RV64 `mtime` register (10 MHz on QEMU `virt` machine).  One tick = 100 ns at 10 MHz.

### Run Protocol

1. **Warmup:** 100 iterations (discarded) — warms QEMU JIT cache
2. **Measurement:** 1,000 iterations per scenario
3. **Statistics:** sort samples → extract `min`, `p50`, `p99`, `max`
4. **Pass/fail:** p99 compared against target; PDR requires p99 ≤ target

### Regression Detection

`scripts/compare-bench-results.sh` compares the current run's p99 against the
rolling median of up to 20 historical runs.  A regression is flagged when a metric
is > 10% above the median for 3 **consecutive** weekly runs (single-run noise is
ignored).  The CI build fails only on sustained regressions.

### Environment

| Parameter | Value |
|-----------|-------|
| Machine | `qemu-system-riscv64 -machine virt -smp 1 -m 128M` |
| Kernel | `riscv64gc-unknown-none-elf` release build |
| BIOS | OpenSBI (default) |
| Runner | GitHub Actions `ubuntu-latest` |

---

## Baseline Measurements

> **Not yet available** — requires the first complete QEMU CI run.
>
> Run `./scripts/dev-setup.sh` then boot via `./run.ps1` and observe `[bench]`
> lines in the serial output to capture initial numbers.

Expected rough order-of-magnitude for QEMU (10 MHz `mtime`):

| Scenario | Expected p50 | Expected p99 | Target |
|----------|-------------|-------------|--------|
| `context_switch` | ~20 µs | ~40 µs | < 100 µs |
| `ipc_send_recv` | ~15 µs | ~30 µs | < 50 µs |
| `syscall_yield` | ~5 µs | ~10 µs | < 10 µs |
| `memory_footprint` | ~3.5 MB | — | < 10 MB |

## Performance Baseline — Status

**Current status: UNMEASURED.** No baseline run has been completed. All values in this document are estimates.

> **Action required (Phase 24)**: Run `/bin/bench` on QEMU, commit results to `.agents/reports/perf-baseline-{date}.txt`, and pin them in CI as regression reference. PDR targets cannot be validated until this is done.

## Spec vs. Implementation Gap

The architecture spec (03-runtime.md) claims IPC at "2–3 CPU cycles via direct function call." The current syscall-based implementation is estimated at 100–1000 cycles per round-trip. The table below tracks the gap:

| Metric | Spec Target | PDR Target (p99) | Estimated Current | Measured |
|--------|------------|-----------------|-------------------|---------|
| IPC round-trip | 2–3 cycles (direct call) | < 50 µs | ~200–500 µs (syscall) | ❌ Not yet |
| Context switch | — | < 100 µs | ~40 µs (estimated) | ❌ Not yet |
| Syscall yield | — | < 10 µs | ~10 µs (estimated) | ❌ Not yet |
| Memory footprint | — | < 10 MB | ~3.5 MB (estimated) | ❌ Not yet |

## Scheduler Impact on Latency

Round-robin 10 ms timeslice means a network packet arriving just after a timeslice boundary waits up to 10 ms before processing. PDR network latency targets (< 10 ms loopback RTT) cannot be met until a preemptive scheduler with priority levels is implemented (Phase 25).

---

## How to Run Locally

```bash
# 1. Build kernel + bench cell
cargo build --release -p Cellos-kernel -p app-bench

# 2. Boot with disk image containing /bin/bench
./run.ps1   # or: bash scripts/run-aarch64.sh for AArch64

# 3. At the shell prompt, spawn bench
Cellos> /bin/bench

# 4. Read serial output for [bench] lines
# JSON lines (parseable by compare-bench-results.sh):
#   {"name":"context_switch","n":1000,"min":42,"p50":55,"p99":90,"max":120}
```

---

## Adding a New Scenario

1. Create `cells/apps/bench/src/scenarios/<name>.rs` implementing `ViBenchmark`
2. Add `pub mod <name>;` to `cells/apps/bench/src/scenarios.rs`
3. Add the scenario to `cells/apps/bench/src/main.rs`
4. Define a PDR target constant and `meets_target()` check
5. Update this document with the new metric row

See `docs/specs/10-testing.md` for the integration test framework that complements these
benchmarks.
