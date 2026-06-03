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

/// Kernel-internal capability bitflags for a task.
///
/// Replaces the single-purpose `can_block_io: bool` (Phase G) with a bitfield
/// that accommodates future kernel-only capabilities without TCB struct changes.
///
/// NOTE: kernel-internal only — distinct from `libs/api` `CapPerms` (file I/O, Law 1).
#[derive(Copy, Clone, Default)]
pub struct KernelPerms(u32);

impl KernelPerms {
    /// Permits raw block-device syscalls (500/501/503). Granted to `/bin/vfs` at spawn.
    pub const BLOCK_IO: Self = Self(1 << 0);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
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

    // Lifecycle
    pub waiters: Vec<usize>,
    pub exit_code: Option<usize>,

    // Async Kernel Support
    pub pending_future: Option<SyscallFuture>,

    /// Kernel capability bitfield. Granted at spawn (e.g. BLOCK_IO for `/bin/vfs`).
    /// Empty for every other cell. Replaces the Phase G `can_block_io: bool`.
    pub kernel_perms: KernelPerms,
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
            waiters: Vec::new(),
            exit_code: None,
            pending_future: None,
            kernel_perms: KernelPerms::default(),
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
