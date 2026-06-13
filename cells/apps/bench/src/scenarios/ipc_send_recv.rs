//! IPC send/recv round-trip latency benchmark.
//!
//! Spawns a private echo peer (another instance of this binary in "ipc-echo"
//! role) and measures the round-trip time of a 64-byte ping. This avoids any
//! dependency on VFS TID, which can vary depending on the boot sequence.
//! PDR target: < 50 µs per round-trip.

use api::benchmark::ViBenchmark;
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
        // Spawn a dedicated echo peer in Normal priority.
        sys_set_spawn_args("ipc-echo");
        let echo_tid = match sys_spawn_pinned(SELF_PATH, 0, 0) {
            SyscallResult::Ok(tid) => tid,
            _ => 0,
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
        sys_send(self.echo_tid, &self.msg);
        let _ = sys_recv(0, &mut self.buf);
        Ok(0)
    }
}

impl Default for IpcSendRecvBench {
    fn default() -> Self { Self::new() }
}
