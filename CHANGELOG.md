## [Unreleased] - v0.2.2-dev (2026-05-30)

### Functional Audit & Verification

A function-based audit replaced the earlier file-existence "100% complete"
claim. Each phase is now verified by a runnable QEMU integration test
(`tests/integration/`, 9/9 green): boot-to-shell, FAT16 mount, interactive
shell echo, Lua REPL, **Lua code execution** (`lua -e print(31337)` → `31337`),
MicroPython REPL, network DHCP lease, GPU framebuffer, and the hot-migration
state-stash round-trip.

Lua now genuinely **executes code**, not just prints a banner. The C heap was
the blocker: picolibc's allocator (incl. the reentrant `_malloc_r` on printf's
float path) grows the heap via `_sbrk`, a toolchain nosys stub returning NULL →
the first chunk write faulted at `addr=0x8`. Fixed with a linker `--wrap=_sbrk`
backed by a static heap in the Lua glue (wins regardless of link order, unlike a
symbol override under `--allow-multiple-definition`). QEMU RAM raised 128 MB →
256 MB so the C runtimes' multi-MB arenas no longer push cumulative frame
allocation into the RAM-ceiling fault.

Real bugs found and fixed during the audit:
- **Lua** did not link (`lua_pcall` is a macro → bind `lua_pcallk`; link picolibc).
- **x86_64 HAL** did not build (AT&T `global_asm!` needs `options(att_syntax)`).
- **RV32 HAL** did not build (rv64 module compiled unconditionally → gated by `target_arch`).
- **Embedded FS** mount failed (`CorruptedFileSystem`): emit FAT16, not an
  undersized FAT32 (< 65525 clusters).
- **Shell input** never reached the command processor: added a direct 16550 RHR
  poll (UART RX IRQ was never delivered to S-mode) and bounded the console buffer.
- **Network**: built the missing net-cell ↔ kernel-NIC bridge — `NetTx`/`NetRx`
  syscalls, `virtio_net::init_driver()` call, RX buffer ≥ `MIN_BUFFER_LEN`, and a
  non-blocking `TryRecv` loop; the net cell now leases 10.0.2.15.
- **GPU**: framebuffer setup no longer hangs — the root cause was a `dma_alloc`
  OOM (now panics instead of spinning) once the heap (32 MB) and FAT16 RAM disk
  left room for the 4 MB framebuffer.
- **Hot migration (Phase 20)**: kernel state-stash primitive
  (`StateStash`/`StateRestore` syscalls, bounded) with a boot self-test.

New kernel↔cell ABI (additive, backward-compatible): syscalls `TryRecv` (7),
`NetTx` (310), `NetRx` (311), `StateStash` (410), `StateRestore` (411).

### Added

**Community Infrastructure (Phase 23)**
- CODE_OF_CONDUCT.md: Contributor Covenant v2.1-based community standards
- dev-setup.sh: fixed Linux disk-gen command; marked executable
- README: added CODE_OF_CONDUCT.md link in contributor section

**Scripting Runtime (Phase 18)**: MicroPython v1.24.1 (547 KB release binary) compiled for RISC-V 64 bare-metal; deployable at VA 0x0E000000; `/bin/python` available in kernel_fs.img via `pyexec_friendly_repl`; custom `gen_genhdr.py` generator bypasses Makefile dependency; Picolibc linked for C runtime (setjmp, frexp, strchr). Complements Lua for lightweight scripting on ViOS cells.

**v1.0 Release Candidate**: All P0/P1/P2 phases complete (23/23 done, Phase 23 ✅); project ready for 1.0 release.

**Complete 8-Task Boot Chain on 128MB RAM**

All services wired and running in QEMU:
- Task 3: VFS Service (RamFS + IPC) at 0x02000000
- Task 4: Config Service (KV store + ViStateTransfer) at 0x00800000
- Task 5: Input Service (US QWERTY + focus dispatch) at 0x04000000 — Phase 14 ✅
- Task 6: Network Service (smoltcp + VirtIO NIC + DHCP) at 0x06000000 — Phase 15 ✅  
- Task 7: Compositor (software blending + VirtIO GPU) at 0x0A000000 — Phase 16 ✅
- Task 8: Shell ("ViOS >") at 0x08000000 — Phase 17 ✅

**Performance**: All bootstrap table entries use release builds (10-100x smaller than debug).
  VFS: 5.7MB→3MB, Net: 4.2MB→~1MB, Shell: 3.2MB→98KB, Compositor: 38KB

**QEMU Configuration** (`run.ps1`):
  - VirtIO block (disk_v3.img with 8 cell entries)
  - VirtIO NIC (user-mode, DHCP assigns 10.0.2.15)
  - VirtIO keyboard (for input service)
  - VirtIO GPU (for compositor rendering)

**Other improvements**:
- Lua `os.execute()`: real implementation via sys_spawn_from_path() (was stub)
- UART input: drains RX_BUFFER AND polls SBI (covers both S-mode and M-mode UART IRQ paths)
- VirtIO input IRQ: calls poll_events() immediately on IRQ fire (reduces input latency)
- Boot log: keystroke events demoted to trace level (no per-key INFO spam)
- gen_disk.ps1: auto-generates both kernel_fs.img and disk_v3.img in one run
- mkfat32.py: full subdirectory support (/bin/, /etc/ in kernel_fs.img)
# ViOS Changelog

All notable changes to ViOS are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased] - v0.2.1-dev

### Added

**Shell (Phase 17)**: Parser (pipes, redirects, background, sequences); job table; 1000-entry history; alias support; built-ins: wc, head, tail, grep, sort, sed, mkdir, rmdir, rm, pwd, uname, free, env, uptime; hot-swap state transfer.

**Network (Phase 15)**: smoltcp 0.11 Cell; DHCP; VirtIO NIC driver; socket IPC API.

**Compositor/GPU (Phase 16)**: Software compositor (z-order, damage, 30 FPS); GpuFlush syscall (300); Surface IPC.

**Input (Phase 14)**: US QWERTY keymap; modifier tracking; focus dispatch.

**Hot Migration (Phase 20)**: ViStateTransfer on Config/Shell/VFS; HotSwap syscall (400); lease auto-revoke; grant chains; scatter/gather IPC (202/203); RecvTimeout (201).

**Scripting (Phase 18)**: Lua 5.4 multi-line REPL + VFS io.open/read/close + shared ostd::repl.

**Benchmarking (Phase 22)**: /bin/bench (4 scenarios); weekly perf CI; regression detection.

**Utilities**: sys-tools (ps,env,uname,date,free,kill,shutdown,hotswap); net-tools (stubs); sort,sed.

**Docs/Infra**: dev-setup.sh+ps1; format-disk.ps1; ROADMAP.md; FAQ.md; hotswap/scripting/vfs/input/display/network API guides; Discussion templates.

### Changed
- Shell help updated with all commands and pipeline/redirect syntax
- VFS IPC: OP_MKDIR(5), OP_RMDIR(6), OP_UNLINK(7)
- ViFileSystem: readdir method added
- CapEntry: lease expiry + grant depth fields

### Fixed
- Lua 9 compiler warnings resolved
- Shell executor now forwards actual parsed arguments to built-in commands

---

## [0.2.0] - 2026-05-01 "Mycelium Alpha"

### Added
- RV64 HAL: SV39, PLIC, SBI, UART; ELF loader with PIE relocation
- Basic shell; VirtIO block (hang fixed); VirtIO keyboard (deadlock fixed)
- AArch64, x86_64, RV32, AArch32 HALs; FileHandle IPC; External ELF loading
- STRIDE threat model; CI/CD with QEMU boot test

[Unreleased]: https://github.com/vi-group/ViCell/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/vi-group/ViCell/releases/tag/v0.2.0

