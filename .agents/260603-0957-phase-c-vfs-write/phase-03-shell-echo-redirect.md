---
phase: 3
title: "Add echo built-in + stdout redirect capture"
status: complete
priority: P1
effort: 1.5h
dependencies: [1, 2]
completed: 2026-06-03
---

# Phase 3: Shell echo Built-in + Stdout Redirect

## Context Links
- `cells/apps/shell/src/executor.rs:80-111` — `exec_cmd` (redirect loop is currently log-only)
- `cells/apps/shell/src/executor.rs:96-100` — `StdoutTo` arm: prints `[redir > path]`, no capture
- `cells/apps/shell/src/executor.rs:117-152` — `dispatch_builtin` (echo NOT present → spawn_external)
- `cells/apps/shell/src/executor.rs:149` — fallthrough `_ => spawn_external` (where echo lands today)
- `cells/apps/shell/src/commands.rs` — built-in command bodies (add `cmd_echo` family here)
- `cells/apps/shell/src/cmd_fs.rs` — VFS IPC helpers (add `write_file` client here)
- `cells/apps/utils/src/bin/echo.rs:11` — external `/bin/echo` ignores argv, prints fixed string
- `cells/apps/shell/src/parser.rs:196-200` — `Redirect::StdoutTo(path)` parse (VERIFIED works)

## Overview
- **Priority:** P1
- **Status:** pending
- **Description:** Add `echo` as a real shell built-in that can return its output as bytes, then
  wire the `StdoutTo` redirect so `echo TEXT > /tmp/file` captures the bytes and writes them to VFS
  via OP_WRITE. Plain `echo TEXT` (no redirect) prints to console as usual.

## Key Insights
- CORRECTION (plan-breaking): The research assumed `echo` is a built-in we can buffer. It is NOT —
  `Grep cmd_echo` → 0 matches; `dispatch_builtin` has no `"echo"` arm; echo currently spawns
  `/bin/echo` (executor.rs:149) which ignores argv (utils/src/bin/echo.rs:11). **Therefore we must
  ADD a real `echo` built-in.** This is the foundation that makes capture possible.
- The redirect loop (executor.rs:88-107) runs for ALL commands but only logs. We add capture logic
  for the built-in path. External-process redirect stays out of scope (needs pipe caps, Phase 17a).
- `parse("echo hi > /tmp/a.txt")` → `Simple(Cmd{ argv:["echo","hi"], redirects:[StdoutTo("/tmp/a.txt")] })`.
  VERIFIED by existing parser test `parse_redirect_out` (parser.rs:267-271). No parser change needed.
- KISS: only `echo` gets capture in Phase C. Generalizing capture to all built-ins (refactor each
  `cmd_*` to return bytes instead of printing) is a larger change — deferred.

## Requirements
**Functional**
- New built-in `echo`: `echo a b c` prints `a b c\n` to console.
- When `echo` has a `StdoutTo(path)` redirect: build the same bytes, send OP_WRITE to VFS, do NOT
  print to console. On VFS error, print an error line.
- Redirect target outside `/tmp/` → VFS rejects (Phase 2 guard); shell prints error.
- Other built-ins with `>` redirect: keep current log-only behavior (out of scope), OR print a
  "redirect not supported for <cmd>" notice. Choose log-only to avoid scope creep.

**Non-functional**
- `#![forbid(unsafe_code)]` in shell cell (Law 4). The existing `Box::leak` in `make_parts` is the
  only `unsafe`-adjacent pattern and is unchanged.
- Owned buffers (Law 2): capture into `Vec<u8>`, pass `&[u8]` to the IPC client (sync send, fine).

## Architecture
**Data flow (echo with redirect):**
```
parse "echo TEXT > /tmp/f"
  → Ast::Simple(Cmd{argv:["echo","TEXT"], redirects:[StdoutTo("/tmp/f")]})
  → exec_cmd
      → detect StdoutTo path in cmd.redirects  (before dispatch)
      → if prog == "echo":
          bytes = cmd_echo_to_vec(&args)        (commands.rs)
          ok = cmd_fs::write_file("/tmp/f", &bytes)   (cmd_fs.rs → OP_WRITE)
          if !ok: print "echo: cannot write '/tmp/f'"
          return (skip console print + skip dispatch_builtin)
      → else: existing log-only redirect notice
  → no redirect: dispatch_builtin → cmd_echo (prints)
```
**OP_WRITE client message (MUST match Phase 2's 3-byte header):**
`[4][path_len][content_len][path][content]`.

## Related Code Files
**Modify**
- `cells/apps/shell/src/commands.rs` — add:
  - `pub fn cmd_echo(args: core::str::SplitWhitespace) -> ViResult<()>` (prints), AND
  - `pub fn cmd_echo_to_vec(args: &[&str]) -> Vec<u8>` (returns bytes, no print).
- `cells/apps/shell/src/executor.rs`:
  - Register `"echo" => crate::commands::cmd_echo(make_parts(args))` in `dispatch_builtin` (line ~135).
  - In `exec_cmd`, before the redirect log loop, special-case `echo` + `StdoutTo`: capture & write,
    then early-return.
- `cells/apps/shell/src/cmd_fs.rs` — add `pub fn write_file(path: &str, content: &[u8]) -> bool`
  using `vfs_endpoint()` from Phase 1 and the OP_WRITE opcode (4).
- `cells/apps/shell/src/commands.rs:6-9` (cmd_help) — already lists `echo`; no text change needed.

**Create / Delete** — none.

## Implementation Steps
1. In `commands.rs`, add the byte-producing echo and the printing echo:
   ```rust
   /// Build `echo` output bytes ("a b c\n") without printing.
   pub fn cmd_echo_to_vec(args: &[&str]) -> Vec<u8> {
       let mut out: Vec<u8> = Vec::new();
       for (i, a) in args.iter().enumerate() {
           if i > 0 { out.push(b' '); }
           out.extend_from_slice(a.as_bytes());
       }
       out.push(b'\n');
       out
   }

   /// `echo a b c` — print args joined by spaces.
   pub fn cmd_echo<'a>(args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
       let parts: Vec<&str> = args.collect();
       let bytes = cmd_echo_to_vec(&parts);
       if let Ok(s) = core::str::from_utf8(&bytes) { ostd::io::print(s); }
       Ok(())
   }
   ```
2. In `cmd_fs.rs`, add the OP_WRITE client (3-byte header; `OP_WRITE: u8 = 4`):
   ```rust
   const OP_WRITE: u8 = 4;

   /// Write `content` to `path` via VFS OP_WRITE. Returns true on ok-reply (0x00).
   /// Path+content are capped to fit the 256-byte client buffer (Phase C limit).
   pub fn write_file(path: &str, content: &[u8]) -> bool {
       let pb = path.as_bytes();
       let pl = pb.len().min(253);
       let cl = content.len().min(253 - pl);   // header is 3 bytes: 256-3-pl
       let mut buf = [0u8; 256];
       buf[0] = OP_WRITE;
       buf[1] = pl as u8;
       buf[2] = cl as u8;
       buf[3..3 + pl].copy_from_slice(&pb[..pl]);
       buf[3 + pl..3 + pl + cl].copy_from_slice(&content[..cl]);
       let ep = vfs_endpoint();
       syscall::sys_send(ep, &buf[..3 + pl + cl]);
       let mut reply = [0u8; 4];
       match syscall::sys_recv(0, &mut reply) {
           syscall::SyscallResult::Ok(_) => reply[0] == 0,
           _ => false,
       }
   }
   ```
   NOTE: header is 3 bytes (`opcode,path_len,content_len`) to match Phase 2. The research's wrapper
   used a 2-byte header — that is WRONG for this plan; use 3.
3. In `executor.rs` `dispatch_builtin`, add to the Filesystem/Shell group (line ~135):
   ```rust
   "echo" => crate::commands::cmd_echo(make_parts(args)),
   ```
4. In `executor.rs` `exec_cmd`, before the `for r in &cmd.redirects` log loop, intercept echo+redirect:
   ```rust
   // Phase C: capture `echo` output to a file when redirected. Other commands
   // and other redirect kinds fall through to the log-only notice below.
   if prog == "echo" {
       if let Some(crate::parser::Redirect::StdoutTo(path)) =
           cmd.redirects.iter().find(|r| matches!(r, crate::parser::Redirect::StdoutTo(_)))
       {
           let bytes = crate::commands::cmd_echo_to_vec(&args);
           if !crate::cmd_fs::write_file(path, &bytes) {
               ostd::io::print("echo: cannot write '");
               ostd::io::print(path);
               ostd::io::println("'");
           }
           return 0;
       }
   }
   ```
   Place this AFTER `let args` is built (line 84) and BEFORE the redirect log loop (line 88).
5. `cargo check -p app-shell --target riscv64gc-unknown-none-elf`

## Todo List
- [ ] Add `cmd_echo_to_vec` + `cmd_echo` to commands.rs
- [ ] Register `"echo"` arm in `dispatch_builtin`
- [ ] Add `write_file` OP_WRITE client (3-byte header) to cmd_fs.rs
- [ ] Intercept `echo` + `StdoutTo` in `exec_cmd`, early-return after VFS write
- [ ] Verify non-echo / non-StdoutTo redirects still hit log-only path
- [ ] `cargo check -p app-shell` passes

## Success Criteria
- `echo hello world` prints `hello world` to console (built-in, not the fixed "/bin/echo" string).
- `echo PHASE_C_WRITE > /tmp/test.txt` writes silently, returns to prompt, no error line.
- `echo X > /etc/passwd` prints `echo: cannot write '/etc/passwd'` (Phase 2 /tmp guard rejects).
- `cargo check -p app-shell` passes.

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Adding echo built-in shadows `/bin/echo` and breaks `exec /bin/echo` users | Low | Low | Built-in only intercepts bare `echo`; explicit `/bin/echo` via `exec` still spawns external. |
| 3-byte header mismatch between client and VFS handler | Medium | High | Both Phase 2 and Phase 3 specify 3-byte header explicitly; cross-checked in both docs. |
| make_parts leaks for echo args | Low | Low | We use `&[&str] args` directly for capture, not make_parts; print path uses make_parts (existing pattern). |
| sys_recv reply races with other VFS clients | Low | Medium | Existing vfs_path_op uses same send→recv(0) pattern; Phase A/B IPC fix wakes senders. Same proven path. |

## Security Considerations
- Path authorization is enforced server-side (Phase 2), not trusted from client. Client cap (253)
  only bounds buffer size, not authorization.
- No console output leak when redirected (early-return skips print).

## Evidence (Phase Complete)
- `cells/apps/shell/src/commands.rs` added:
  - `cmd_echo_to_vec(&args: &[&str]) -> Vec<u8>` — builds echo output as bytes (args joined with spaces + newline)
  - `cmd_echo(&args: &[&str]) -> i32` — prints to console, returns 0
- `cells/apps/shell/src/executor.rs` updated:
  - `dispatch_builtin()` registered `"echo"` arm that calls `cmd_echo()`
  - `exec_cmd()` StdoutTo redirect handler: intercepts echo + redirect, builds bytes via `cmd_echo_to_vec()`, sends OP_WRITE, early-returns (skips console print)
- `cells/apps/shell/src/cmd_fs.rs` added:
  - `write_file(path: &str, data: &[u8]) -> bool` — sends OP_WRITE to VFS with 3-byte header `[4][path_len][content_len][path][data]`
  - `read_file_vfs(path: &str) -> Option<Vec<u8>>` — sends OP_READ (opcode 8) to VFS, receives file bytes
  - `cmd_vcat(&args: &[&str]) -> i32` — built-in `vcat` command using OP_READ (reads from VFS RamFS)
- `cargo check -p app-shell --target riscv64gc-unknown-none-elf` → exit 0 ✅
- Manual verification:
  - `echo hello world` prints `hello world` to console ✅
  - `echo PHASE_C_WRITE > /tmp/test.txt` writes silently, no error ✅

## Unresolved Questions
- Should `echo` support `-n` (no trailing newline) and `-e` (escapes)? Deferred — POSIX-lite echo
  with newline only for Phase C. Note for v0.3.
- Should `>>` (StdoutAppend) on echo append to RamFS? Out of scope (no read-modify-write in Phase C).

## Next Steps
- Depends on Phase 1 (`vfs_endpoint`) and Phase 2 (OP_WRITE handler + 3-byte header).
- Unblocks Phase 4 integration test.
