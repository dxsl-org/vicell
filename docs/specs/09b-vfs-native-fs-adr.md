# ADR: Native Filesystem for G2 (/srv backend)

**Date**: 2026-06-11 | **Status**: Accepted | **Authors**: ViCell core team

---

## Decision

Port **RedoxFS** (MIT licence, ~10 K LOC Rust) as the `/srv` backend for G2,
activated when the NVMe driver ships. Do not implement a custom CoW B-tree.

---

## Context

G1 (robot/embedded) persists data to:
- `/data` — littlefs on NAND/eMMC P4, power-loss safe, no cross-platform tools.
- `/mnt/sd` — FAT32 on SD card P1, PC-interoperable, no journal.

G2 (server/PC workload, C930/P870 + NVMe) needs:
- **Copy-on-write (CoW)** to support zero-copy snapshots for containers and VMs.
- **Checksums** for silent-data-corruption detection on multi-TB NVMe.
- **Crash recovery** without fsck (journal/log-structured replaying at mount).
- **Large files & directories** (hundreds of GB; FAT32 4 GB limit is a non-starter).

---

## Options Evaluated

| Option | LOC | Licence | CoW | Checksum | no_std | Verdict |
|--------|-----|---------|-----|----------|--------|---------|
| **RedoxFS port** | ~10 K | MIT | ✅ | ✅ | Needs shim | ✅ Chosen |
| Custom CoW B-tree | ~30-40 K | N/A | ✅ | ✅ | ✅ | ❌ YAGNI |
| TFS (TheFileSystem) | ~5 K | MIT | ✅ | ✅ | Partial | ❌ Upstream dead (~2018) |
| ext4 FFI (e2fsprogs) | ~300 K | GPL-2 | ❌ | ✅ | ❌ | ❌ GPL conflict |
| BtrFS FFI | ~200 K | GPL-2 | ✅ | ✅ | ❌ | ❌ GPL conflict |

### Why RedoxFS wins

- **Production-proven**: runs in Redox OS on RISC-V hardware; not a toy.
- **Manageable scope**: ~10 K lines fits a single G2 sprint (4–6 weeks).
- **Pure Rust**: avoids the FFI toolchain complexity of C libs (unlike littlefs2).
- **`no_std` path is clear**: replace `std::collections::BTreeMap` with
  `alloc::collections::BTreeMap`; map `std::io::Error` → `ViError`; wrap
  file handles as `ViFileHandle`. Redox already maintains an `alloc`-only build
  for its kernel space.
- **MIT licence**: compatible with ViCell's MIT licence.

### Why custom CoW B-tree was rejected

Filesystem correctness is notoriously hard to prove (fsck edge cases, torn
writes, ABA in B-tree splits). Writing a new CoW B-tree would take 12–18 months
of careful implementation and testing, not 4–6 weeks. Redox has done that work.

---

## Architecture (G2 implementation guide)

```
/srv
  └── RedoxFsBackend (cells/services/vfs/src/backend_redoxfs.rs)
        └── NvmeBlockAdapter
              └── DMA Grant block API (sys_grant_blk_read/write, G2 syscalls)
```

**no_std adaptation checklist** (start here when NVMe is ready):

1. Fork `redox-os/redoxfs` at a tagged release; add as a `[patch]` or vendor subtree.
2. Enable `default-features = false, features = ["alloc"]` (Redox already has this).
3. Replace `std::collections::*` → `alloc::collections::*` throughout.
4. Replace `std::io::{Error, Read, Write, Seek}` → ViCell equivalents from `libs/ostd`.
5. Implement `NvmeBlockAdapter: BlockDevice` backed by the G2 DMA Grant API
   (see `docs/specs/02-memory.md §5` — large-buffer IPC via `sys_grant_blk_read`).
6. Mount lazily: wait for NVMe driver `ServiceReady` notification before calling
   `Filesystem::open()`. Use the same `LookupService` IPC pattern as the net cell.
7. Surface `RedoxFsBackend` through `FsBackend` trait. No changes to dispatch, quota,
   or access layers — the MountTable handles routing transparently.

**Block-region gate**: add `MANIFEST_FLAG_PART_SRV` (bit 8) in `libs/api/src/manifest.rs`
when the NVMe partition is defined. Requires **Law 1 confirmation** (2× user approval).

---

## Trigger Conditions

Implement RedoxFsBackend when **all three** are true:
1. NVMe driver cell ships and passes block read/write integration tests.
2. A G2 server board (C930/P870) is available for hardware validation.
3. The G2 benchmark target (`<100 µs read latency`) is defined and measurable.

Until then `/srv` serves a no-op `StubBackend`
(see `cells/services/vfs/src/backend_stub.rs`).

---

## Consequences

- `StubBackend` at `/srv` ensures G1 cells that try to open `/srv/…` get a clean
  empty-response instead of the `/` RamFS fallback — early detection of misrouted
  paths.
- When RedoxFsBackend lands, the mount line in `manager.rs` is the only change
  needed in the VFS service; all other layers are unaffected.
- If RedoxFS upstream diverges significantly before G2, re-evaluate against
  the custom option; the 4× LOC estimate is a G2-budget concern, not a G1 concern.
