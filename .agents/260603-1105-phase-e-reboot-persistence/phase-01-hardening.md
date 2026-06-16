# Phase 1 — Hardening (SeekFrom cap + sector-range guard)

## Context Links
- `cells/services/vfs/src/block_stream.rs:83-93` (Seek impl, verified)
- `kernel/src/task/syscall.rs:1068-1091` (BlkRead/BlkWrite handlers, verified)
- `kernel/src/loader/disk_layout.rs:22` (`pub const CELL_TABLE_BASE_LBA: u64 = 82_000`, verified)

## Overview
- **Priority:** P1 (safety — closes two Phase D review findings)
- **Status:** complete (2026-06-03)
- **Depends on:** nothing (fully independent)
- Two small defensive fixes. No happy-path behavior change.

## Key Insights
- `SeekFrom::Current(n)` with negative `n` underflows: `pos=100, n=-200` → `(100i64 + -200) as u64` = huge u64 → seek to a random far sector. Verified the cast at `block_stream.rs:87`.
- `SeekFrom::End(_)` already returns `Err(())` (`block_stream.rs:90`) — leave as-is.
- BlkRead/BlkWrite pass `sector` straight to `viVirtIOBlk` with no upper bound (`syscall.rs:1075,1087`). A cell can address LBA ≥ `CELL_TABLE_BASE_LBA` (82_000) and corrupt the bootstrap table that the loader reads at boot.
- `CELL_TABLE_BASE_LBA` is `pub const` and reachable as `crate::loader::disk_layout::CELL_TABLE_BASE_LBA` (same module already referenced at `syscall.rs:1094` via `MAX_CELL_PATH`).

## Requirements
- **Functional:** negative-result `Current` seek returns `Err(())`; any sector ≥ 82_000 in BlkRead/BlkWrite is rejected (returns 0 = failure to caller).
- **Non-functional:** no allocation, no panic, `#![forbid(unsafe_code)]` already holds in vfs cell (the guard is pure arithmetic).

## Data flow
```
fatfs Seek(Current(n))
  → pos as i64 + n
    → < 0?  → Err(())            [NEW: rejected]
    → >= 0? → result as u64      [unchanged]

cell ecall(500/501, sector, buf)
  → ViCell_syscall_dispatch maps to Syscall::BlkRead/BlkWrite
    → handle_syscall:
        sector >= CELL_TABLE_BASE_LBA? → Ok(0)   [NEW: rejected, no driver call]
        else → validate_user_buf → viVirtIOBlk.read/write_sector
```

## Related Code Files
- **Modify:** `cells/services/vfs/src/block_stream.rs` (line 87)
- **Modify:** `kernel/src/task/syscall.rs` (BlkRead handler ~1068, BlkWrite handler ~1080)
- **Create / delete:** none

## Implementation Steps

### 1a. Fix SeekFrom::Current underflow
Replace `block_stream.rs:87`:
```rust
            fatfs::SeekFrom::Current(n) => (self.pos as i64 + n) as u64,
```
with:
```rust
            fatfs::SeekFrom::Current(n) => {
                // Reject seeks before byte 0 — a negative result would otherwise
                // wrap to a huge u64 and seek to an arbitrary far sector.
                let result = self.pos as i64 + n;
                if result < 0 { return Err(()); }
                result as u64
            }
```

### 1b. Add sector-range cap to BlkRead (syscall.rs ~1068)
Insert the guard as the FIRST statement inside the `Syscall::BlkRead { sector, buf_ptr } =>` arm, before `validate_user_buf`:
```rust
        Syscall::BlkRead { sector, buf_ptr } => {
            // Reject any sector at/after the cell bootstrap table; a runaway FAT
            // offset must never read kernel-owned LBAs. Returns 0 = failure.
            if sector >= crate::loader::disk_layout::CELL_TABLE_BASE_LBA {
                return Ok(0);
            }
            validate_user_buf(buf_ptr, 512, MAX_USER_BUF)?;
            // ... existing body unchanged ...
```

### 1c. Add the same cap to BlkWrite (syscall.rs ~1080)
```rust
        Syscall::BlkWrite { sector, buf_ptr } => {
            // Reject any sector at/after the cell bootstrap table; prevents a
            // cell from corrupting the loader's table. Returns 0 = failure.
            if sector >= crate::loader::disk_layout::CELL_TABLE_BASE_LBA {
                return Ok(0);
            }
            validate_user_buf(buf_ptr, 512, MAX_USER_BUF)?;
            // ... existing body unchanged ...
```
> Note: the handler arms return `Result<usize, SyscallError>` (see `Ok(0)` / `Ok(1)` already used at `syscall.rs:1076-1077`), so `return Ok(0);` is type-correct here.

## Todo List
- [ ] 1a: SeekFrom::Current underflow guard in `block_stream.rs:87`
- [ ] 1b: BlkRead sector cap in `syscall.rs`
- [ ] 1c: BlkWrite sector cap in `syscall.rs`
- [ ] `cargo check -p ViCell-kernel -p service-vfs --target riscv64gc-unknown-none-elf`

## Success Criteria
- Compiles clean (above check).
- The 40 MB FAT image lives entirely below 81_920; legitimate `/data` access (sector < 82_000) is unaffected — `vfs_fat16_write_read` still passes.

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| Guard rejects a legit FAT sector | Low | Med | FAT image ends at 81_920 < 82_000; verified at `disk_layout.rs:20-22`. |
| `CELL_TABLE_BASE_LBA` not in scope at syscall.rs | Low | Low | Same path already used at `syscall.rs:1094`; verified `pub const`. |

## Security Considerations
- Closes a privilege-boundary hole: a U-mode cell could previously write LBAs the loader trusts. Now bounded.

## Evidence (Complete)
- `cells/services/vfs/src/block_stream.rs:87` — SeekFrom::Current now validates `result >= 0` before cast
- `kernel/src/task/syscall.rs` (BlkRead ~1072, BlkWrite ~1084) — both guards cap sector < CELL_TABLE_BASE_LBA
- All 14 integration tests pass; `vfs_fat16_write_read` regression-tested

## Next Steps
- None block this; it is a leaf phase. Land before or in parallel with Phase 2's `syscall.rs` edit (apply Phase 1 first — disjoint line ranges).
