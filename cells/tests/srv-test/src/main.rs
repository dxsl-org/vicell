//! RedoxFS /srv integration test cell.
//!
//! Verifies the VFS /srv backend (RedoxFS on MBR partition P5) by exercising
//! five scenarios over IPC, then writes a persistence marker for the
//! two-boot persistence integration test.
//!
//! Requires a disk with P5 formatted by `scripts/mksrv-img.sh`.
//!
//! Expected output (all pass):
//!   [srv-test] S1 mount: PASS
//!   [srv-test] S2 write+read: PASS
//!   [srv-test] S3 listdir: PASS
//!   [srv-test] S4 mkdir: PASS
//!   [srv-test] S5 unlink: PASS
//!   [srv-test] PERSIST_WRITE_DONE
//!   [srv-test] ALL TESTS PASSED

#![no_std]
#![no_main]
extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, LookupService];

fn vfs_tid() -> usize {
    use api::syscall::service;
    loop {
        if let Some(tid) = ostd::syscall::sys_lookup_service(service::VFS) {
            return tid;
        }
        ostd::task::yield_now();
    }
}

static PASSED: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);

fn vfs_req(req: &api::ipc::VfsRequest<'_>) -> api::ipc::VfsResponse<'static> {
    let mut send_buf = [0u8; api::ipc::IPC_BUF_SIZE];
    let n = api::ipc::encode(req, &mut send_buf).map(|s| s.len()).unwrap_or(0);
    ostd::syscall::sys_send(vfs_tid(), &send_buf[..n]);
    let buf: &'static mut [u8; api::ipc::IPC_BUF_SIZE] =
        alloc::boxed::Box::leak(alloc::boxed::Box::new([0u8; api::ipc::IPC_BUF_SIZE]));
    match ostd::syscall::sys_recv(0, buf) {
        ostd::syscall::SyscallResult::Ok(_) =>
            api::ipc::decode::<api::ipc::VfsResponse>(buf)
                .unwrap_or(api::ipc::VfsResponse::Err(0xFE)),
        _ => api::ipc::VfsResponse::Err(0xFD),
    }
}

fn pass(label: &str) {
    PASSED.fetch_add(1, Ordering::Relaxed);
    ostd::io::print("[srv-test] ");
    ostd::io::print(label);
    ostd::io::println(": PASS");
}

fn fail(label: &str, note: &str) {
    FAILED.fetch_add(1, Ordering::Relaxed);
    ostd::io::print("[srv-test] ");
    ostd::io::print(label);
    ostd::io::print(": FAIL (");
    ostd::io::print(note);
    ostd::io::println(")");
}

// ── Scenario implementations ─────────────────────────────────────────────────

/// S1: /srv root must stat as a directory (confirms P5 was opened by VFS).
fn test_s1_mount() {
    match vfs_req(&api::ipc::VfsRequest::Stat("/srv")) {
        api::ipc::VfsResponse::Stat { is_dir: true, .. } => pass("S1 mount"),
        api::ipc::VfsResponse::Err(c) => fail("S1 mount", if c == 0xFF { "VFS uninit" } else { "Err" }),
        _ => fail("S1 mount", "not a dir"),
    }
}

/// S2: Write a file and read it back using ReadAsync + Poll.
fn test_s2_write_read() {
    let content = b"ViCell RedoxFS";

    match vfs_req(&api::ipc::VfsRequest::Write {
        path: "/srv/test.txt", content,
    }) {
        api::ipc::VfsResponse::Ok => {}
        _ => { fail("S2 write+read", "write failed"); return; }
    }

    let handle = match vfs_req(&api::ipc::VfsRequest::ReadAsync { path: "/srv/test.txt" }) {
        api::ipc::VfsResponse::PendingHandle(h) => h,
        _ => { fail("S2 write+read", "ReadAsync no handle"); return; }
    };

    match vfs_req(&api::ipc::VfsRequest::Poll { handle }) {
        api::ipc::VfsResponse::Data(d) if d == content => pass("S2 write+read"),
        api::ipc::VfsResponse::Data(_) => fail("S2 write+read", "wrong content"),
        _ => fail("S2 write+read", "Poll failed"),
    }
}

/// S3: Directory listing of /srv must include test.txt (written by S2).
fn test_s3_listdir() {
    match vfs_req(&api::ipc::VfsRequest::ListDir("/srv")) {
        api::ipc::VfsResponse::Data(bytes) => {
            if bytes.windows(10).any(|w| w == b"f:test.txt") {
                pass("S3 listdir");
            } else {
                fail("S3 listdir", "test.txt not listed");
            }
        }
        _ => fail("S3 listdir", "ListDir failed"),
    }
}

/// S4: Create /srv/subdir, verify it's a directory, then clean it up.
fn test_s4_mkdir() {
    match vfs_req(&api::ipc::VfsRequest::Mkdir("/srv/subdir")) {
        api::ipc::VfsResponse::Ok => {}
        _ => { fail("S4 mkdir", "mkdir failed"); return; }
    }

    match vfs_req(&api::ipc::VfsRequest::Stat("/srv/subdir")) {
        api::ipc::VfsResponse::Stat { is_dir: true, .. } => pass("S4 mkdir"),
        _ => { fail("S4 mkdir", "stat not dir"); return; }
    }

    // Clean up so re-runs (e.g. persistence test boot 2) have a clean state.
    let _ = vfs_req(&api::ipc::VfsRequest::Rmdir("/srv/subdir"));
}

/// S5: Write /srv/tmp.txt, unlink it, confirm it no longer exists.
fn test_s5_unlink() {
    match vfs_req(&api::ipc::VfsRequest::Write {
        path: "/srv/tmp.txt", content: b"x",
    }) {
        api::ipc::VfsResponse::Ok => {}
        _ => { fail("S5 unlink", "write failed"); return; }
    }

    match vfs_req(&api::ipc::VfsRequest::Unlink("/srv/tmp.txt")) {
        api::ipc::VfsResponse::Ok => {}
        _ => { fail("S5 unlink", "unlink failed"); return; }
    }

    match vfs_req(&api::ipc::VfsRequest::Stat("/srv/tmp.txt")) {
        api::ipc::VfsResponse::Err(_) => pass("S5 unlink"),
        _ => fail("S5 unlink", "file still exists"),
    }
}

// ── Persistence marker ───────────────────────────────────────────────────────

/// Check if a persist marker from a previous boot is present.
/// Prints "[srv-test] PERSIST_READ_OK" if found so the integration harness
/// can verify two-boot persistence without additional infrastructure.
fn check_persist_marker() {
    match vfs_req(&api::ipc::VfsRequest::Stat("/srv/persist.txt")) {
        api::ipc::VfsResponse::Stat { is_dir: false, size, .. } if size > 0 => {
            ostd::io::println("[srv-test] PERSIST_READ_OK");
        }
        _ => {}
    }
}

/// Write the persistence marker so the next boot can verify it.
fn write_persist_marker() {
    let _ = vfs_req(&api::ipc::VfsRequest::Write {
        path: "/srv/persist.txt",
        content: b"ViCell-persist-ok",
    });
    ostd::io::println("[srv-test] PERSIST_WRITE_DONE");
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[no_mangle]
pub fn main() {
    ostd::io::println("[srv-test] Starting RedoxFS /srv test suite...");

    // Check for persist marker from a previous boot BEFORE running any tests
    // (so the integration harness sees PERSIST_READ_OK early in the output).
    check_persist_marker();

    test_s1_mount();
    test_s2_write_read();
    test_s3_listdir();
    test_s4_mkdir();
    test_s5_unlink();

    write_persist_marker();

    let passed = PASSED.load(Ordering::Relaxed);
    let failed = FAILED.load(Ordering::Relaxed);

    ostd::io::print("[srv-test] Results: ");
    ostd::io::print_usize(passed as usize);
    ostd::io::print(" PASS, ");
    ostd::io::print_usize(failed as usize);
    ostd::io::println(" FAIL");

    if failed == 0 {
        ostd::io::println("[srv-test] ALL TESTS PASSED");
        ostd::syscall::sys_exit(0);
    } else {
        ostd::io::println("[srv-test] FAILURES DETECTED");
        ostd::syscall::sys_exit(1);
    }
}
