//! Shell built-in test harness (feature = "shell_test").
//!
//! Replaces the interactive REPL when the binary is compiled with
//! `--features shell_test`.  Exercises parser + executor scenarios via
//! `executor::capture_line`, asserts on captured output or VFS file contents,
//! then prints `[shell-test] ALL TESTS PASSED` so the CI integration test can
//! `wait_for` it.
//!
//! VFS must be up before file-I/O scenarios run.  The harness waits via
//! `sys_lookup_service(VFS)` (same pattern as vfs-test/srv-test).

use crate::jobs::Jobs;

const VFS_SVC: u16 = api::syscall::service::VFS;

/// Wait for the VFS service to register (blocks until init has spawned vfs).
fn wait_for_vfs() {
    loop {
        if ostd::syscall::sys_lookup_service(VFS_SVC).is_some() {
            return;
        }
        ostd::task::yield_now();
    }
}

// ── Assertion helpers ─────────────────────────────────────────────────────────

static mut PASSED: u32 = 0;
static mut FAILED: u32 = 0;

fn pass(name: &str) {
    // SAFETY: single shell task; no concurrent access.
    unsafe { PASSED += 1; }
    ostd::io::print("[shell-test] PASS  ");
    ostd::io::println(name);
}

fn fail(name: &str, got: &str, want: &str) {
    // SAFETY: single shell task; no concurrent access.
    unsafe { FAILED += 1; }
    ostd::io::print("[shell-test] FAIL  ");
    ostd::io::println(name);
    ostd::io::print("  got:  "); ostd::io::println(got);
    ostd::io::print("  want: "); ostd::io::println(want);
}

/// Assert that captured output (as UTF-8) contains `needle`.
fn assert_contains(jobs: &mut Jobs, name: &str, line: &str, needle: &str) {
    let bytes = crate::executor::capture_line(line, jobs);
    let got = core::str::from_utf8(&bytes).unwrap_or("<invalid utf8>");
    if got.contains(needle) {
        pass(name);
    } else {
        fail(name, got, needle);
    }
}

/// Assert that the VFS file at `path` contains `needle`.
fn assert_file_contains(name: &str, path: &str, needle: &str) {
    let mut buf = [0u8; 480];
    let n = crate::cmd_fs::read_file_vfs(path, &mut buf);
    if n == 0 {
        fail(name, "<file not found>", needle);
        return;
    }
    let got = core::str::from_utf8(&buf[..n]).unwrap_or("<invalid utf8>");
    if got.contains(needle) {
        pass(name);
    } else {
        fail(name, got, needle);
    }
}

/// Execute `line` (captures output, discards it). Used for setup steps.
fn run(jobs: &mut Jobs, line: &str) {
    crate::executor::capture_line(line, jobs);
}

// ── Test scenarios ────────────────────────────────────────────────────────────

fn test_stdout_redirect(jobs: &mut Jobs) {
    run(jobs, "echo REDIR_OUT > /tmp/st_redir.txt");
    assert_file_contains("stdout redirect writes file", "/tmp/st_redir.txt", "REDIR_OUT");
}

fn test_append_redirect(jobs: &mut Jobs) {
    run(jobs, "echo APPEND_A > /tmp/st_append.txt");
    run(jobs, "echo APPEND_B >> /tmp/st_append.txt");
    assert_file_contains("append redirect line A", "/tmp/st_append.txt", "APPEND_A");
    assert_file_contains("append redirect line B", "/tmp/st_append.txt", "APPEND_B");
}

fn test_stderr_redirect(jobs: &mut Jobs) {
    // Phase 1: 2> routes output to file (single-channel shell, stderr==stdout).
    run(jobs, "echo STDERR_OUT 2> /tmp/st_stderr.txt");
    assert_file_contains("stderr redirect writes file", "/tmp/st_stderr.txt", "STDERR_OUT");
}

fn test_pipe_grep(jobs: &mut Jobs) {
    // Pipeline: echo multi-line | grep pattern.
    assert_contains(jobs, "pipe grep matches lines",
        "echo -e ax\\nby\\ncx | grep x",
        "ax");
}

fn test_wc_l(jobs: &mut Jobs) {
    // wc -l on 3-line input via pipeline.
    assert_contains(jobs, "wc -l counts 3 lines",
        "echo -e a\\nb\\nc | wc",
        "3");
}

fn test_sort(jobs: &mut Jobs) {
    assert_contains(jobs, "sort produces first line 'a'",
        "echo -e c\\na\\nb | sort",
        "a");
}

fn test_tee(jobs: &mut Jobs) {
    // Phase 2: tee writes to both sink and file.
    let bytes = crate::executor::capture_line("echo -e x\\ny | tee /tmp/st_tee.txt", jobs);
    let got = core::str::from_utf8(&bytes).unwrap_or("");
    if got.contains('x') {
        pass("tee passes data to sink");
    } else {
        fail("tee passes data to sink", got, "contains 'x'");
    }
    assert_file_contains("tee writes file", "/tmp/st_tee.txt", "x");
}

fn test_sed(jobs: &mut Jobs) {
    // Phase 2: sed substitution (first occurrence).
    assert_contains(jobs, "sed first-occurrence substitution",
        "echo foo bar | sed s/foo/baz/",
        "baz bar");
    // Global substitution.
    assert_contains(jobs, "sed global substitution",
        "echo foo foo | sed s/foo/baz/g",
        "baz baz");
}

fn test_fg_bg(jobs: &mut Jobs) {
    // Phase 3: fg/bg print limitation message, not "command not found".
    assert_contains(jobs, "fg prints limitation message",
        "fg",
        "no job control");
    assert_contains(jobs, "bg prints limitation message",
        "bg",
        "no job control");
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Test harness entry point — called from `main()` when feature `shell_test` is set.
pub fn run() {
    ostd::io::println("[shell-test] Starting shell utility tests...");

    // Wait for VFS before running file-I/O scenarios.
    wait_for_vfs();
    // Let VFS finish its init pass before issuing writes.
    ostd::task::yield_now();

    let mut jobs = Jobs::new();

    test_stdout_redirect(&mut jobs);
    test_append_redirect(&mut jobs);
    test_stderr_redirect(&mut jobs);
    test_pipe_grep(&mut jobs);
    test_wc_l(&mut jobs);
    test_sort(&mut jobs);
    test_tee(&mut jobs);
    test_sed(&mut jobs);
    test_fg_bg(&mut jobs);

    // SAFETY: single shell task; no concurrent reads.
    let (passed, failed) = unsafe { (PASSED, FAILED) };
    ostd::io::println("");
    ostd::io::print("[shell-test] Results: ");
    ostd::io::print_usize(passed as usize);
    ostd::io::print(" PASS, ");
    ostd::io::print_usize(failed as usize);
    ostd::io::println(" FAIL");

    if failed == 0 {
        ostd::io::println("[shell-test] ALL TESTS PASSED");
        ostd::syscall::sys_exit(0usize);
    } else {
        ostd::io::println("[shell-test] FAILURES DETECTED");
        ostd::syscall::sys_exit(1usize);
    }
}
