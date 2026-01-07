use alloc::string::String;
use alloc::vec::Vec; 
use crate::hal::arch::Context;
use crate::hal::arch::ViTrapFrame;
use alloc::collections::BTreeMap;
// use crate::fs::{Inode, DirStream};
use alloc::sync::Arc;
use alloc::boxed::Box;
use types::*;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    /// Sleeping until a specific monotonic time (ticks/ms)
    Sleeping { until: usize },
    /// Blocked waiting to send a message to `target_id`.
    /// Stores the message pointer and length temporarily.
    Sending { target: usize, msg_ptr: VAddr, msg_len: usize },
    /// Blocked waiting to receive a message.
    /// `mask`: Filter mask (e.g., from specific sender or any).
    Recv { mask: usize, buf_ptr: VAddr, buf_len: usize },
    /// This task has finished running.
    Terminated,
    /// Blocked on a Futex wait.
    /// `addr`: The address being waited on.
    FutexWait { addr: VAddr },
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
    pub id: usize,    // Logic Lease ID (Index)
    pub ptr: VAddr,   // Address in Task's space
    pub len: usize,   // Length
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
use api::fs::ViFile;

// File Handle for Stateful IO
pub struct FileHandle(pub Box<dyn ViFile + Send + Sync>);

// Manual Debug
impl core::fmt::Debug for FileHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "FileHandle")
    }
}

/// Task Control Block (TCB)
#[allow(dead_code)]
#[derive(Debug)] // Removed Clone
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
        }
    }
    
    pub fn add_lease(&mut self, ptr: VAddr, len: usize, attributes: LeaseAttributes) -> usize {
        let id = self.next_lease_id;
        self.next_lease_id += 1;
        
        let lease = Lease { id, ptr, len, attributes };
        
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
        self.grant_table.insert(id, GrantEntry { ptr, len, flags, sender_id });
        id
    }

    pub fn get_grant(&self, id: usize) -> Option<&GrantEntry> {
        self.grant_table.get(&id)
    }

    pub fn remove_grant(&mut self, id: usize) -> Option<GrantEntry> {
        self.grant_table.remove(&id)
    }
}
