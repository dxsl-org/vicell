use crate::hal::arch::Context;
use crate::hal::arch::ViTrapFrame;
use alloc::string::String;
use alloc::vec::Vec;
// use alloc::boxed::Box;
// use alloc::sync::Arc;
use types::*;

use api::fs::{BoxFuture, FileResult};

#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    /// Sleeping until a specific monotonic time (ticks/ms)
    Sleeping {
        until: usize,
    },
    /// Blocked waiting to send a message to `target_id`.
    /// Stores the message pointer and length temporarily.
    Sending {
        target: usize,
        msg_ptr: VAddr,
        msg_len: usize,
    },
    /// Blocked waiting to receive a message.
    ///
    /// `mask`: sender filter (0 = any sender).
    /// `deadline`: optional monotonic tick count after which the kernel wakes
    ///   this task with `ViError::Timeout`.  `None` = wait indefinitely.
    Recv {
        mask: usize,
        buf_ptr: VAddr,
        buf_len: usize,
        deadline: Option<u64>,
    },
    /// This task has finished running.
    Terminated,
    /// Blocked on a Futex wait.
    /// `addr`: The address being waited on.
    FutexWait {
        addr: VAddr,
    },
    /// Waiting for another task to exit (Join).
    Waiting {
        target: usize,
    },
    /// Polling an async future (e.g. syscall)
    Polling,
    /// Blocked in `WaitForEvent(mask, timeout)`.  Woken by `waker::signal_net_rx()`
    /// (or equivalent) when any bit in `mask` fires, or when `deadline` ticks pass.
    /// `deadline = None` means block indefinitely.
    WaitEvent {
        mask: u32,
        deadline: Option<u64>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseAttributes(pub u32);

impl LeaseAttributes {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub id: usize,  // Logic Lease ID (Index)
    pub ptr: VAddr, // Address in Task's space
    pub len: usize, // Length
    pub attributes: LeaseAttributes,
}

#[derive(Debug, Clone)]
pub struct GrantEntry {
    pub ptr: VAddr,
    pub len: usize,
    pub flags: u32,
    pub sender_id: usize,
}

// File Handle for Stateful IO
pub use api::fs::FileHandle;

/// Enum to hold the different types of futures a task might be waiting on.
pub enum SyscallFuture {
    FileRead(usize, BoxFuture<'static, FileResult<usize>>), // fd, future
                                                            // Add other syscall futures here (FileWrite, Connect, etc.)
}


/// Task Control Block (TCB)
#[allow(dead_code)]
pub struct Task {
    pub id: usize,
    pub cell_id: CellId, // OWNER CELL
    pub name: String,
    pub state: TaskState,
    pub context: Context,
    pub trap_frame: ViTrapFrame,
    pub allowed_drivers: Vec<usize>,
    // Maps LeaseID -> Lease
    pub leases: alloc::collections::BTreeMap<usize, Lease>,
    // Next available Lease ID
    pub next_lease_id: usize,

    // Grant Table (Zero-Copy IPC)
    // Maps GrantID -> GrantEntry
    pub grant_table: alloc::collections::BTreeMap<usize, GrantEntry>,
    pub next_grant_id: usize,

    // Maps FD -> FileHandle
    pub open_files: alloc::collections::BTreeMap<usize, FileHandle>,
    // The Task ID that this task is currently handling a request FROM (for Reply).
    pub current_caller: Option<usize>,
    // Last Reply Value received
    pub reply_value: Option<usize>,
    // Current Working Directory
    pub cwd: String,
    // Stack management
    pub kernel_stack: Option<super::stack::Stack>,
    pub user_stack: Option<super::stack::Stack>,
    /// Frames mapped for this cell's ELF segments, freed when the Task is dropped
    /// (reaped). Without it, a cell's code/data frames leak on every death.
    pub segment_mem: Option<super::stack::CellSegments>,

    // Lifecycle
    pub waiters: Vec<usize>,
    pub exit_code: Option<usize>,
    /// Death-notification queue (NotifyOnExit): tids of watched tasks that died
    /// while this watcher was NOT parked in Recv. Drained (highest-priority) by the
    /// next `Recv` so a supervisor never misses a child death during a respawn.
    /// Each entry is `(dead_tid, exit_reason)` — the reason is delivered as the recv
    /// payload (the NotifyOnExit contract) so a supervisor can apply a restart policy
    /// (e.g. transient = restart only on abnormal exit). `exit_reason` is the exit code
    /// for a clean exit (0) or `usize::MAX` for a fault / watchdog kill.
    pub pending_deaths: Vec<(usize, usize)>,

    /// Exit reason for a death notification delivered to a watcher that was PARKED in
    /// `Recv` (set by `exit_task`). It is written into the watcher's recv buffer when its
    /// `Recv` resumes — in the watcher's own syscall context, where writing a USER buffer
    /// is valid (SSTATUS.SUM). It must NOT be written from `exit_task`/the trap context,
    /// where an S-mode store to a USER page faults. `None` = the wake was a real message.
    pub pending_exit_reason: Option<usize>,

    // Async Kernel Support
    pub pending_future: Option<SyscallFuture>,

    /// Raw block-device access (BlkRead/BlkWrite/BlkFlush).  Granted at spawn for `/bin/vfs`.
    pub block_io_cap: Option<super::cap::BlockIoCap>,
    /// Network transmit/receive (NetTx/NetRx).  Granted at spawn for `/bin/net`.
    pub network_cap:  Option<super::cap::NetworkCap>,
    /// Cell spawning + hot-swap (SpawnFromPath/SpawnPinned/HotSwap).
    /// Granted at spawn for `/bin/init` and `/bin/shell`.
    pub spawn_cap:    Option<super::cap::SpawnCap>,
    /// RISC-V H-extension CSR access for VMM cells.
    /// Granted when manifest declares `hypervisor = true` AND the firmware reported H-ext.
    pub hypervisor_cap: Option<super::cap::HypervisorCap>,

    /// MMIO peripheral access (GPIO or UART).  `true` if the ELF manifest
    /// declared `gpio` or `uart` cap; grants access to `sys_request_mmio`.
    pub mmio_cap: bool,

    /// Scheduling priority tier.  Higher value = higher priority.
    /// See `api::TaskPriority` for the three defined levels.
    pub priority: u8,

    /// Per-Cell syscall allowlist.  Each bit corresponds to a syscall via
    /// `api::ViSyscall::allowlist_bit()`.  `u64::MAX` = permit all (default,
    /// used when the Cell ELF does not embed a `__ViCell_syscalls` section).
    pub syscall_allowlist: u64,

    /// Watchdog: consecutive 10 ms scheduler ticks this task has been Running
    /// WITHOUT voluntarily blocking. Incremented each tick it is found Running in
    /// `pick_next`, reset to 0 the moment it blocks (Recv/Send/Sleep/etc). A
    /// runaway (infinite loop, never yields) climbs until it crosses the watchdog
    /// budget and is terminated — preventing livelock ("alive but paralyzed").
    pub run_ticks: u32,

    /// Cumulative count of `RecvTimeout` deadlines this task has missed (the awaited
    /// message did not arrive in time). For an RT control loop this is its missed-cycle
    /// count. Observability only — surfaced via the audit ring ([`crate::audit`]); the
    /// scheduler does not act on it (RT enforcement is hardware-data-gated).
    pub deadline_misses: u32,

    /// One-shot latch: set when this task has already emitted an `RtCpuOverrun` warning
    /// for the current non-yielding episode, so the early-warning audit fires once per
    /// episode (not every tick). Reset to false whenever the task voluntarily blocks.
    pub rt_overrun_warned: bool,

    /// Liveness-heartbeat deadline (absolute `system_ticks`). `Some(d)` means the cell
    /// opted into heartbeating and must call `Heartbeat` again before tick `d`, else the
    /// scheduler terminates it as HUNG (silent-hang detection — see `pick_next`). `None`
    /// = heartbeat disabled (the default; most cells don't opt in).
    pub heartbeat_deadline: Option<u64>,
}

impl Task {
    pub fn new(id: usize, cell_id: CellId, name: &str, allowed_drivers: Vec<usize>) -> Self {
        Self {
            id,
            cell_id,
            name: String::from(name),
            state: TaskState::Ready,
            context: Context::default(),
            trap_frame: ViTrapFrame::default(),
            allowed_drivers,
            leases: alloc::collections::BTreeMap::new(),
            next_lease_id: 1, // Start efficiently
            grant_table: alloc::collections::BTreeMap::new(),
            next_grant_id: 1,
            open_files: alloc::collections::BTreeMap::new(),
            current_caller: None,
            reply_value: None,
            cwd: String::from("/"),
            kernel_stack: None,
            user_stack: None,
            segment_mem: None,
            waiters: Vec::new(),
            exit_code: None,
            pending_deaths: Vec::new(),
            pending_exit_reason: None,
            pending_future: None,
            block_io_cap:   None,
            network_cap:    None,
            spawn_cap:      None,
            hypervisor_cap: None,
            mmio_cap:       false,
            priority: api::TaskPriority::Normal as u8,
            syscall_allowlist: u64::MAX, // permit-all until ELF section is read
            run_ticks: 0,
            deadline_misses: 0,
            rt_overrun_warned: false,
            heartbeat_deadline: None,
        }
    }

    pub fn add_lease(&mut self, ptr: VAddr, len: usize, attributes: LeaseAttributes) -> usize {
        let id = self.next_lease_id;
        self.next_lease_id += 1;

        let lease = Lease {
            id,
            ptr,
            len,
            attributes,
        };

        self.leases.insert(id, lease);
        id
    }

    pub fn get_lease(&self, id: usize) -> Option<&Lease> {
        self.leases.get(&id)
    }

    pub fn revoke_lease(&mut self, id: usize) {
        self.leases.remove(&id);
    }

    // --- Grant Table Methods ---
    pub fn add_grant(&mut self, ptr: VAddr, len: usize, flags: u32, sender_id: usize) -> usize {
        let id = self.next_grant_id;
        self.next_grant_id += 1;
        self.grant_table.insert(
            id,
            GrantEntry {
                ptr,
                len,
                flags,
                sender_id,
            },
        );
        id
    }

    pub fn get_grant(&self, id: usize) -> Option<&GrantEntry> {
        self.grant_table.get(&id)
    }

    pub fn remove_grant(&mut self, id: usize) -> Option<GrantEntry> {
        self.grant_table.remove(&id)
    }
}
