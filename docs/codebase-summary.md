# Cellos Codebase Summary

**Project**: Cellos (Jarvis Hybrid OS)
**Version**: 0.2.1-dev (Mycelium Era)
**Language**: Rust (nightly, `no_std`)
**Crates**: ~52 active workspace members
**Last Updated**: 2026-06-05 (Phase 23 complete)

---

## Quick Stats

| Area | Crates | Key Highlights |
|------|--------|---------------|
| Kernel | 1 | ~8,700 LOC; HotSwap, scatter/gather IPC, lease caps, VirtIO VA→PA fix |
| HAL | 10 | RV64 full, AArch64 + x86_64 full (Ring-3 smoke), RV32 + AArch32 stubs |
| Libraries | 3 | `types`, `api` (display/input/hotswap APIs), `ostd` (repl, gpu, hotswap wrappers) |
| Apps | 8+ | shell (parser+executor+45+ built-ins+$(cmd)+args), bench, sys-tools, net-tools (6 bins), utils |
| Drivers | 6 | disk, gpu, input, net (VirtIO NIC), serial, wasm |
| Services | 6 | vfs (RamFS+FAT16+10 opcodes), compositor (30 FPS), net (TCP/UDP/DNS), input, config, power |
| Runtimes | 2 | Lua 5.4 (verified, network bindings), MicroPython 1.24.1 (verified, vnet module) |

---

## Directory Structure

```
Cellos/
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
│   ├── net-tools/          ping, curl (HTTP GET), nc (TCP relay), wget (downloader), httpd (file server), mqtt (skeleton)
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
│   ├── vfs/                RamFS + FAT16 + 10 IPC opcodes (GET_FILE/LIST_DIR/STAT/WRITE/READ/MKDIR/RMDIR/UNLINK/RMDIR_RECURSIVE/APPEND) + ViStateTransfer
│   │   └── src/            mount.rs, quota.rs, handle_table.rs, block_stream.rs
│   ├── compositor/         Software blending, z-order, damage tracking, GPU flush
│   │   └── src/            surface_table.rs, z_order.rs, render.rs
│   ├── net/                smoltcp TCP/IP + UDP + DNS resolver + DHCP + socket IPC (11 opcodes)
│   │   └── src/            interface.rs, socket_table.rs, socket_state.rs, dhcp.rs, poll_driver.rs
│   ├── input/              US QWERTY translator, modifier state, focus dispatcher + ViStateTransfer
│   │   └── src/            layout_us_qwerty.rs, modifier_state.rs, dispatcher.rs
│   ├── config/             KV store + ViStateTransfer (schema v1)
│   └── power/              Power management stub
│
├── cells/runtimes/ (2 crates)
│   ├── lua/                Lua 5.4 REPL (multi-line, history, VFS I/O FFI, network bindings, ViStateTransfer)
│   │   ├── src/ffi.rs      Lua C API bindings
│   │   ├── src/bindings_io.rs  Cellos_io_open/read/close, Cellos_os_execute
│   │   ├── src/bindings_net.rs  vnet.udp_send/recv, vnet.resolve, DNS FFI
│   │   └── src/repl_session.rs Multi-line REPL + <eof> continuation detection
│   └── micropython/        MicroPython 1.24.1 REPL (256KB heap, VFS I/O, vnet module, ViStateTransfer)
│       ├── src/ffi.rs      MicroPython C API bindings
│       ├── src/builtins.rs sys, os, math, json, vnet modules
│       ├── src/modvnet.c   TCP/UDP socket API, vnet_dns.c DNS resolution, modvnet_udp.c UDP sockets
│       └── src/repl.rs     Interactive REPL session
│
├── tests/integration/      QEMU-driven test suite (QemuRunner harness, 30/30 tests)
│   ├── lib.rs              QemuRunner, spawn_echo_server, spawn_http_server, spawn_mqtt_broker, wait_for
│   ├── boot.rs             9 named tests: boots_to_shell_prompt, shell_run_echo, shell_read_file, shell_run_utils, lua_repl_runs, lua_code_executes, micropython_repl_runs, network_dhcp_lease, gpu_framebuffer_renders
│   └── (additional scenarios): FAT16 persistence, HotSwap, network TCP/UDP/DNS, Lua/MicroPython module tests
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
