# good-first-issue Backlog — Phase 23

Create these on GitHub with labels: `good-first-issue` + `area:<x>` + `difficulty:<S|M|L>`.

---

## Issue 1 — Add `wc` utility (word/line/char count)

**Labels:** `good-first-issue`, `area:utils`, `difficulty:S`

**Context:**
`cells/apps/utils/` contains small standalone utilities built as Cells. A `wc`
implementation would read from stdin (or a file path) and print word, line, and
byte counts, matching the POSIX `wc` contract at a basic level.

**Acceptance criteria:**
- [ ] `wc /path/to/file` prints `<lines> <words> <bytes> <filename>`
- [ ] `wc` without args reads from stdin until EOF
- [ ] Lives in `cells/apps/utils/src/wc.rs` or a new `cells/apps/utils/wc/` sub-crate
- [ ] `cargo check --workspace` stays clean
- [ ] At least one unit test

**Files of interest:**
- `cells/apps/utils/src/` — existing utility structure
- `libs/ostd/src/fs.rs` — `File::open` + `read_to_end`
- `libs/ostd/src/console.rs` — `println!`

---

## Issue 2 — Add `tee` utility (stdin → stdout + file)

**Labels:** `good-first-issue`, `area:utils`, `difficulty:S`

**Context:**
`tee` copies stdin to both stdout and a named file. It exercises the VFS write path
and is a useful shell composition primitive.

**Acceptance criteria:**
- [ ] `echo hello | tee /tmp/out.txt` writes `hello` to `/tmp/out.txt` and stdout
- [ ] Handles file-write errors gracefully (prints error, exits non-zero)
- [ ] `cargo check --workspace` clean

**Files of interest:**
- `cells/apps/utils/src/`
- `libs/ostd/src/fs.rs` — `File::write_all` (currently stub — can be wired to VFS IPC)
- `cells/services/vfs/src/main.rs` — `OP_WRITE` handler

---

## Issue 3 — Expand `docs/10-testing.md`: how to add a new integration test

**Labels:** `good-first-issue`, `area:docs`, `difficulty:S`

**Context:**
`docs/10-testing.md` describes the testing strategy but doesn't yet explain how a
contributor adds a new integration test using the QEMU harness.

**Acceptance criteria:**
- [ ] New section "Adding a new integration test" in `docs/10-testing.md`
- [ ] Explains how to use `QemuRunner` from `tests/integration/harness.rs`
- [ ] Shows a minimal example test function
- [ ] Explains how to run it (`--target x86_64-...` required)

**Files of interest:**
- `docs/10-testing.md`
- `tests/integration/harness.rs`
- `tests/integration/ring3_smoke.rs` — example to reference

---

## Issue 4 — Add `head` and `tail` utilities

**Labels:** `good-first-issue`, `area:utils`, `difficulty:S`

**Context:**
`head -n N file` prints the first N lines; `tail -n N file` prints the last N.
Both are standard shell composition tools.

**Acceptance criteria:**
- [ ] `head -n 5 /path/to/file` prints first 5 lines
- [ ] `tail -n 5 /path/to/file` prints last 5 lines
- [ ] Default N = 10 when `-n` is omitted
- [ ] `cargo check --workspace` clean

**Files of interest:**
- `cells/apps/utils/src/`
- `libs/ostd/src/fs.rs`

---

## Issue 5 — Add `alias` builtin to the shell

**Labels:** `good-first-issue`, `area:shell`, `difficulty:M`

**Context:**
The shell (`cells/apps/shell/`) currently has no alias support. Adding `alias
name=command` and `unalias name` would significantly improve ergonomics.

**Acceptance criteria:**
- [ ] `alias ll='ls -l'` stores the alias in a `BTreeMap<String, String>`
- [ ] Running `ll` in the shell expands to `ls -l` before dispatch
- [ ] `alias` with no args lists all defined aliases
- [ ] `unalias ll` removes an alias
- [ ] Aliases are not persisted across reboots (in-memory only for v1.0)

**Files of interest:**
- `cells/apps/shell/src/` — command dispatch
- `libs/ostd/src/` — string utilities

---

## Issue 6 — Add integration test for `cp -r /foo /bar`

**Labels:** `good-first-issue`, `area:tests`, `difficulty:M`

**Context:**
`tests/integration/` contains QEMU-driven tests. A test for recursive directory copy
would exercise the VFS mkdir + write paths and drive the shell's `cp -r` flow.

**Acceptance criteria:**
- [ ] New file `tests/integration/vfs_copy.rs`
- [ ] Uses `QemuRunner` from `harness.rs`
- [ ] Creates a directory, writes a file, copies recursively, asserts destination exists
- [ ] Test marked `#[ignore]` until FAT32 backing lands (VirtIO-FAT milestone)

**Files of interest:**
- `tests/integration/harness.rs`
- `tests/integration/multi_cell.rs` — pattern to follow
- `cells/services/vfs/src/main.rs` — VFS IPC opcodes

---

## Issue 7 — Add dead-link checker to CI for `docs/`

**Labels:** `good-first-issue`, `area:docs`, `difficulty:M`

**Context:**
`docs/ARCHITECTURE.md` and other docs occasionally reference files or sections that
have been moved or renamed. A lightweight CI step (e.g. `mlc` or a Bash script)
would catch broken local links automatically.

**Acceptance criteria:**
- [ ] New job `docs-links` in `.github/workflows/ci.yml`
- [ ] Runs a link checker on `docs/**/*.md` and `*.md` at root
- [ ] Fails on broken local file links; ignores external URLs (too noisy)
- [ ] Existing broken links fixed as part of this PR

**Files of interest:**
- `.github/workflows/ci.yml`
- `docs/` — all markdown files

---

## Issue 8 — Stub keyboard layout file for DE / FR layout

**Labels:** `good-first-issue`, `area:input`, `difficulty:M`

**Context:**
`kernel/src/task/drivers/input_map.rs` hard-codes US QWERTY scancodes. Adding a
`LayoutDe` or `LayoutFr` struct that implements a `KeyLayout` trait would be a
clean first step towards i18n layout support.

**Acceptance criteria:**
- [ ] New `KeyLayout` trait in `kernel/src/task/drivers/input_map.rs` (or a new
  `kernel/src/task/drivers/key_layout.rs`)
- [ ] `LayoutUs` and `LayoutDe` (at minimum) implement the trait
- [ ] Existing US scancode table wrapped as `LayoutUs`
- [ ] `cargo check --workspace` clean; no regression in existing key handling

**Files of interest:**
- `kernel/src/task/drivers/input_map.rs`
- `kernel/src/task/drivers/input_map.rs` — existing table

---

## Issue 9 — Add a Lua example script to `/examples/`

**Labels:** `good-first-issue`, `area:lua`, `difficulty:S`

**Context:**
`cells/runtimes/lua/` integrates Lua 5.4. There are no example `.lua` scripts in the
repo. Adding a small set of examples (`hello.lua`, `fib.lua`, `readfile.lua`) with
a README would lower the barrier for contributors exploring the Lua runtime.

**Acceptance criteria:**
- [ ] New directory `examples/lua/`
- [ ] At least 3 `.lua` scripts with comments explaining what they demonstrate
- [ ] `examples/lua/README.md` explaining how to run them via the ViCell Lua Cell

**Files of interest:**
- `cells/runtimes/lua/src/`
- `docs/05-application.md` — scripting runtime docs

---

## Issue 10 — Clean up `#[allow(dead_code)]` attributes in VFS service

**Labels:** `good-first-issue`, `area:vfs`, `difficulty:S`

**Context:**
`cells/services/vfs/src/mount.rs`, `quota.rs`, and `handle_table.rs` carry top-level
`#![allow(dead_code)]` suppression added as a placeholder until the write path is
wired. With `OP_MKDIR`/`OP_RMDIR`/`OP_UNLINK` now live, some of these can be removed.

**Acceptance criteria:**
- [ ] Remove `#![allow(dead_code)]` from modules where all items are now used
- [ ] If items are still dead, add `// reason:` inline to each `#[allow(dead_code)]`
  attribute (per code-standards.md rule on attribute comments)
- [ ] `cargo clippy --workspace` passes without new warnings

**Files of interest:**
- `cells/services/vfs/src/mount.rs`
- `cells/services/vfs/src/quota.rs`
- `cells/services/vfs/src/handle_table.rs`
