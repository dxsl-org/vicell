# ViOS Codebase Summary

**Project**: ViOS (Jarvis Hybrid OS)
**Version**: 0.2.1-dev (Mycelium Era)
**Language**: Rust (nightly, `no_std`)
**Crates**: ~40 active workspace members
**Last Updated**: 2026-06-03

---

## Quick Stats

| Area | Crates | Key Highlights |
|------|--------|---------------|
| Kernel | 1 | ~8,700 LOC; HotSwap, scatter/gather IPC, lease caps |
| HAL | 10 | RV64 full, AArch64 + x86_64 full (Ring-3 smoke), RV32 + AArch32 stubs |
| Libraries | 3 | `types`, `api` (display/input/hotswap APIs), `ostd` (repl, gpu, hotswap wrappers) |
| Apps | 8 | shell (parser+executor+aliases+jobs), bench, sys-tools, net-tools, utils |
| Drivers | 6 | disk, gpu, input, net (VirtIO NIC), serial, wasm |
| Services | 6 | vfs (RamFS+IPC), compositor (30 FPS), net (smoltcp+DHCP), input, config, power |
| Runtimes | 2 | Lua 5.4 (verified), MicroPython 1.24.1 (verified) |

---

## Directory Structure

```
vios/
├── kernel/src/
│   ├── main.rs             Boot orchestration
│   ├── cell/
│   │   ├── registry.rs     Cell lifecycle + VA range allocation
│   │   ├── cap_registry.rs Capability table (lease expiry, grant depth)
│   │   ├── hotswap.rs      5-step live Cell replacement (Phase 20)
│   │   └── metadata.rs     CellHeader metadata
│   ├── memory/
│   │   ├── frame.rs        Bitmap frame allocator
│   │   ├── heap.rs         Kernel heap
│   │   ├── paging.rs       SV39 page tables
│   │   └── tests.rs        Stress tests (10 K alloc/free, multi-size)
│   ├── loader/
│   │   ├── elf.rs          ELF parser + segment loader
│   │   ├── reloc.rs        R_RISCV_RELATIVE relocation engine
│   │   ├── early.rs        Boot-time disk reader (before VFS)
│   │   ├── disk_layout.rs  Shared disk layout constants
│   │   └── elf_tests.rs    10 boot-time ELF + relocation tests
│   ├── task/
│   │   ├── scheduler.rs    Round-robin scheduler
│   │   ├── syscall.rs      HotSwap, GpuFlush, SendGather, RecvScatter, RecvTimeout
│   │   ├── tcb.rs          Task control block (Recv deadline field)
│   │   ├── tests.rs        11 scheduler + state-transition tests
│   │   ├── ipc_test.rs     IPC scenario stubs
│   │   ├── stack.rs        Kernel stack management
│   │   └── drivers/
│   │       ├── virtio_blk.rs  VirtIO block driver
│   │       ├── virtio_gpu.rs  VirtIO GPU (RESOURCE_CREATE_2D, flush)
│   │       ├── virtio_input.rs VirtIO keyboard/mouse
│   │       ├── virtio_net.rs  VirtIO NIC (Phase 15)
│   │       ├── fb_console.rs  Framebuffer text console
│   │       ├── uart.rs        NS16550A UART
│   │       ├── input_map.rs   Scancode → ASCII
│   │       └── ...
│   ├── fs/fat.rs           FAT32 via `fatfs` crate
│   └── fs.rs               FS facade
│
├── hal/ (10 crates)
│   ├── core/               Feature-gated arch facade
│   ├── traits/             6 pure trait crates (arch, paging, interrupt, timer, uart, display)
│   └── arch/
│       ├── riscv/          RV64 full + RV32 trait impls
│       ├── arm/            AArch64 full + AArch32 trait impls
│       └── x86/            x86_64 full impl (IDT, GDT, LAPIC, syscall/sysret)
│
├── libs/ (3 crates)
│   ├── types/src/lib.rs    VAddr, PAddr, CellId, ViError, DirEntry + 10 host unit tests
│   ├── api/src/
│   │   ├── syscall.rs      26 ViSyscall variants + ABI tests (5 host tests)
│   │   ├── fs.rs           ViFileSystem (open/read/write/mkdir/rmdir/unlink/readdir)
│   │   ├── input.rs        InputEvent, KeyEvent, KeySym, Modifiers (Phase 14)
│   │   ├── display.rs      Rect, PixelFormat, SurfaceCap, compositor opcodes (Phase 16)
│   │   ├── benchmark.rs    ViBenchmark + BenchReport (p50/p99 + JSON)
│   │   ├── hotswap.rs      ViStateTransfer trait
│   │   ├── cap.rs          CapId, CapPerms
│   │   └── net.rs          ViTcpStack, IpEndpoint
│   └── ostd/src/
│       ├── syscall.rs      sys_gpu_flush, sys_hotswap, sys_recv_timeout, sys_send_gather, …
│       ├── fs.rs           File::open/read/close (cap-based)
│       └── repl.rs         Shared readline + 500-entry history ring buffer
│
├── cells/apps/ (8 crates)
│   ├── init/               Spawns VFS→Config→Shell from /bin/
│   ├── shell/              Full REPL: parser (pipe/redir/bg/seq), executor, alias, jobs, history
│   │   └── src/            parser.rs, executor.rs, jobs.rs, history.rs, aliases.rs,
│   │                       cmd_fs.rs (wc/head/tail/grep/mkdir/rm), cmd_sys.rs, state_transfer.rs
│   ├── bench/              4-scenario benchmark (ctx-switch, IPC, syscall, footprint)
│   ├── utils/              wc, head, tail, grep, sort, sed, cat, ls, cp, mv, rm, mkdir, touch, echo
│   ├── sys-tools/          ps, env, uname, date, free, kill, shutdown, hotswap
│   ├── net-tools/          ping, curl, nc, wget (stubs for Phase 15 data-path)
│   ├── hello/              Minimal ELF smoke test
│   └── test-isolation/     Capability isolation test cell
│
├── cells/drivers/ (6 crates)
│   ├── disk/               VirtIO block passthrough
│   ├── gpu/                GPU cell (flush_rect + fill_rect helpers)
│   ├── input/              VirtIO input passthrough
│   ├── net/                VirtIO NIC cell wrapper
│   ├── serial/             UART driver cell
│   └── wasm/               WebAssembly runtime stub
│
├── cells/services/ (6 crates)
│   ├── vfs/                RamFS + OP_MKDIR/RMDIR/UNLINK/STAT IPC + ViStateTransfer
│   │   └── src/            mount.rs, quota.rs, handle_table.rs
│   ├── compositor/         Software blending, z-order, damage tracking, GPU flush
│   │   └── src/            surface_table.rs, z_order.rs, render.rs
│   ├── net/                smoltcp TCP/IP + DHCP + socket IPC
│   │   └── src/            interface.rs, socket_table.rs, dhcp.rs, poll_driver.rs
│   ├── input/              US QWERTY translator, modifier state, focus dispatcher + ViStateTransfer
│   │   └── src/            layout_us_qwerty.rs, modifier_state.rs, dispatcher.rs
│   ├── config/             KV store + ViStateTransfer (schema v1)
│   └── power/              Power management stub
│
├── cells/runtimes/ (2 crates)
│   ├── lua/                Lua 5.4 REPL (multi-line, history, VFS I/O FFI, ViStateTransfer)
│   │   ├── src/ffi.rs      Lua C API bindings
│   │   ├── src/bindings_io.rs  vios_io_open/read/close, vios_os_execute
│   │   └── src/repl_session.rs Multi-line REPL + <eof> continuation detection
│   └── micropython/        MicroPython 1.24.1 REPL (256KB heap, VFS I/O, ViStateTransfer)
│       ├── src/ffi.rs      MicroPython C API bindings
│       ├── src/builtins.rs sys, os, math, json modules
│       └── src/repl.rs     Interactive REPL session
│
├── tests/integration/      QEMU-driven test stubs (QemuRunner harness)
│   ├── harness.rs          Boot QEMU, inject input, grep serial output
│   ├── ring3_smoke.rs      Banner + Ring-3 hello + shell prompt
│   ├── multi_cell.rs       init→config→vfs→shell chain
│   ├── input_dispatch.rs   Key injection + shell echo
│   ├── network_loopback.rs DHCP + TCP loopback
│   ├── compositor_basic.rs GPU init + surface no-panic
│   └── hotswap_shell.rs    Live shell upgrade + history preservation
│
├── scripts/
│   ├── dev-setup.sh / .ps1 One-command setup (Linux/macOS/Windows)
│   ├── format-disk.ps1     FAT32 disk image generator
│   ├── compare-bench-results.sh  Rolling-median perf regression detector
│   └── measure-coverage.sh LLVM coverage script
│
└── docs/
    ├── vfs-api.md, input-api.md, display-api.md, network-api.md
    ├── hotswap-guide.md, scripting-guide.md, performance-report.md
    ├── README.md, faq.md, getting-started.md
    ├── project-roadmap.md, project-changelog.md, project-overview-pdr.md
    └── specs/  (00-context … 11-shell — design specifications)

```

---

## Key Design Principles

1. **Single Address Space (SAS)** — all Cells share one virtual address space; no TLB flush on IPC
2. **Cellular isolation** — `#![forbid(unsafe_code)]` in every Cell crate; HAL-only unsafe
3. **Capability model** — capabilities have optional lease expiry + grant-depth enforcement
4. **Hot-swap** — `ViStateTransfer` on shell/config/vfs; 5-step live Cell replacement
5. **Law 2 (Owned Buffers)** — no `&mut [u8]` across `async` boundaries
6. **Law 5 (No mod.rs)** — `foo.rs` parallel to `foo/` everywhere

---

## Syscall Surface (selected)

| ID | Name | Description |
|----|------|-------------|
| 0–3 | Send/Recv/Call/Reply | Core IPC |
| 12 | SpawnFromPath | Load cell ELF from /bin/ |
| 13–15 | OpenCap/ReadCap/CloseCap | Capability-based file I/O |
| 201 | RecvTimeout | IPC with monotonic-tick deadline |
| 202–203 | SendGather/RecvScatter | Scatter/gather IPC (Phase 20) |
| 300 | GpuFlush | Blit pixel rect to VirtIO GPU |
| 400 | HotSwap | Live Cell replacement |
