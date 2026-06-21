//! IPC send/recv round-trip latency benchmark.
//!
//! Spawns a private echo peer (another instance of this binary in "ipc-echo"
//! role) and measures the round-trip time of a 64-byte ping. This avoids any
//! dependency on VFS TID, which can vary depending on the boot sequence.
//! PDR target: < 50 µs per round-trip.

use api::{benchmark::ViBenchmark, TaskPriority};
use ostd::syscall::{sys_send, sys_recv, sys_set_spawn_args, sys_spawn_pinned, SyscallResult};

// Use bench-probe (VA 0x19000000) so the echo peer doesn't collide with the
// orchestrator's pages (VA 0x18000000) in the shared SAS page table.
const SELF_PATH: &str = "/bin/bench-probe";

pub struct IpcSendRecvBench {
    echo_tid: usize,
    msg: [u8; 64],
    buf: [u8; 64],
}

impl IpcSendRecvBench {
    pub fn new() -> Self {
        // Spawn echo peer at Normal priority (1) so it isn't starved when the
        // RT under_load scenario has 3 Normal-priority load cells spinning.
        sys_set_spawn_args("ipc-echo");
        let echo_tid = match sys_spawn_pinned(SELF_PATH, api::TaskPriority::Normal as u8, 0) {
            SyscallResult::Ok(tid) => tid,
            _ => {
                ostd::io::println("FATAL: Failed to spawn bench-probe. Missing in disk image?");
                ostd::syscall::sys_exit(1);
                0
            }
        };
        // Yield a few times so the echo peer reaches its recv loop.
        for _ in 0..20 { ostd::task::yield_now(); }
        let mut msg = [0u8; 64];
        msg[0] = 0x42; // ping
        Self { echo_tid, msg, buf: [0u8; 64] }
    }
}

impl ViBenchmark for IpcSendRecvBench {
    fn name(&self) -> &'static str { "ipc_send_recv" }

    fn run_once(&mut self) -> api::ViResult<u64> {
        if self.echo_tid == 0 {
            return Ok(0); // echo not spawned — bench not at /bin/bench
        }
        let r1 = sys_send(self.echo_tid, &self.msg);
        let r2 = sys_recv(0, &mut self.buf);
        if !matches!(r1, ostd::syscall::SyscallResult::Ok(_)) || !matches!(r2, ostd::syscall::SyscallResult::Ok(_)) {
            ostd::io::println(&alloc::format!("IPC err: send={:?} recv={:?}", r1, r2));
        }
        Ok(0)
    }
}

impl Drop for IpcSendRecvBench {
    fn drop(&mut self) {
        // Kill the echo peer so its VA (0x19000000) is freed before RT tests
        // try to spawn bench-probe in a different role. Without this, the
        // ipc-echo instance stays alive → VA collision → RT scenarios all fail.
        if self.echo_tid != 0 {
            let _ = ostd::syscall::sys_force_exit(self.echo_tid);
        }
    }
}

impl Default for IpcSendRecvBench {
    fn default() -> Self { Self::new() }
}
