//! CapId-keyed socket handle table for the net service cell.
//!
//! Maps kernel-issued `CapId`s (u64) to smoltcp `SocketHandle`s so that
//! any consumer cell can reference an open socket across IPC calls without
//! exposing smoltcp-internal handles.

extern crate alloc;

use alloc::collections::BTreeMap;
use smoltcp::iface::SocketHandle;
use types::ViError;
use crate::socket_state::SocketState;

/// Maximum simultaneous sockets (including the DHCP management socket).
pub const MAX_SOCKETS: usize = 18; // 16 user + 1 DHCP + 1 ARP

/// Maps a `CapId` to a smoltcp `SocketHandle` and connection state.
#[derive(Default)]
pub struct SocketTable {
    entries: BTreeMap<u64, SocketHandle>,
    states:  BTreeMap<u64, SocketState>,
    next_cap: u64,
}

impl SocketTable {
    pub fn new() -> Self {
        Self { entries: BTreeMap::new(), states: BTreeMap::new(), next_cap: 1 }
    }

    /// Allocate a new `CapId` and associate it with `handle`.
    ///
    /// # Errors
    /// Returns `ViError::OutOfMemory` if `MAX_SOCKETS` is already reached.
    pub fn insert(&mut self, handle: SocketHandle) -> Result<u64, ViError> {
        if self.entries.len() >= MAX_SOCKETS {
            return Err(ViError::OutOfMemory);
        }
        let cap = self.next_cap;
        self.next_cap += 1;
        self.entries.insert(cap, handle);
        self.states.insert(cap, SocketState::Created);
        Ok(cap)
    }

    /// Look up the smoltcp `SocketHandle` for a given `CapId`.
    pub fn get(&self, cap: u64) -> Option<SocketHandle> {
        self.entries.get(&cap).copied()
    }

    /// Read the connection state for `cap`.
    pub fn get_state(&self, cap: u64) -> Option<SocketState> {
        self.states.get(&cap).copied()
    }

    /// Update the connection state for `cap`.
    pub fn set_state(&mut self, cap: u64, state: SocketState) {
        if self.entries.contains_key(&cap) {
            self.states.insert(cap, state);
        }
    }

    /// Remove a socket from the table (called on close).
    pub fn remove(&mut self, cap: u64) -> Option<SocketHandle> {
        self.states.remove(&cap);
        self.entries.remove(&cap)
    }
}
