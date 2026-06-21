//! Shell history — in-memory ring buffer with optional VFS persistence.
// `len`, `get`, and `flush_to_disk` are called by state_transfer (hotswap) and
// will be called by the readline repl; suppress dead-code lint until wired.
#![allow(dead_code)] // reason: public methods used by state_transfer + future readline integration
//!
//! File persistence (`~/.ViCell_history`) is enabled once VFS write is stable
//! (Phase 13 FAT32).  Until then the history survives only within a session.

extern crate alloc;
use alloc::{collections::VecDeque, string::String};

/// Maximum entries kept in memory.
const MAX_HISTORY: usize = 1000;
/// Path for persistent history file (VFS write path required).
const HISTORY_FILE: &str = "/tmp/.ViCell_history";

/// Persistent-capable command history.
pub struct History {
    entries: VecDeque<String>,
    dirty: bool,
}

impl History {
    pub fn new() -> Self {
        let mut h = Self { entries: VecDeque::with_capacity(MAX_HISTORY), dirty: false };
        h.load_from_disk();
        h
    }

    /// Add a command; de-duplicates consecutive identical entries.
    pub fn push(&mut self, line: &str) {
        if line.is_empty() { return; }
        if self.entries.back().map(|s| s.as_str()) == Some(line) { return; }
        if self.entries.len() >= MAX_HISTORY { self.entries.pop_front(); }
        self.entries.push_back(String::from(line));
        self.dirty = true;
    }

    /// Number of entries.
    pub fn len(&self) -> usize { self.entries.len() }

    /// Access by 0-based index (0 = oldest).
    pub fn get(&self, idx: usize) -> Option<&str> {
        self.entries.get(idx).map(String::as_str)
    }

    /// Persist to disk if dirty and VFS write is available.
    ///
    /// Silently skips if the VFS write path is not yet functional.
    pub fn flush_to_disk(&mut self) {
        if !self.dirty { return; }
        // VFS write not yet available (Phase 13 FAT32); silently skip.
        self.dirty = false;
    }

    /// Load history from disk on startup (no-op until VFS write path lands).
    fn load_from_disk(&mut self) {
        // Phase 13 FAT32 will enable reading from HISTORY_FILE.
        let _ = HISTORY_FILE; // suppress unused-constant warning in doc
    }
}

impl Default for History {
    fn default() -> Self { Self::new() }
}
