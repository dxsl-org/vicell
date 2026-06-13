---
name: compositor-cursor-implementation
description: Track B software cursor sprite + CI integration test implementation details
metadata:
  type: project
---

Phase 01 + Phase 04 of Track B implemented 2026-06-13.

**Why:** Make the compositor render a visible cursor and prove it with a CI test.

**Worktree compositor version differs from plan assumptions:**
- render.rs in worktree is simpler (no staging buffer, time-based render loop at 30fps, not damage-driven)
- render_frame signature was `(fb, table, z_order)` — extended to `(fb, table, z_order, extra_dirty, cursor_x, cursor_y)`
- No `has_damage()` method on SurfaceTable in worktree; time-based render handles cursor dirty via pending_dirty

**integration test lib.rs in worktree has no monitor field** — needed to add:
- `monitor: Option<TcpStream>` field to QemuRunner
- `boot_with_pointer()` constructor — uses `-qmp tcp:...,server,nowait` (QEMU is QMP server, we connect to it)
- `send_qemu_mouse_abs(x, y)` method — QMP `input-send-event` with abs axis events

**Key design note on QMP direction:**
- Serial: we bind → QEMU connects (client)
- QMP: QEMU binds (`server,nowait`) → we connect after startup

**How to apply:** When adding more QMP-based tests, use the same `boot_with_pointer` + `send_qemu_mouse_abs` pattern. Note `-qmp tcp:addr:port,server,nowait` is the correct flag format.
