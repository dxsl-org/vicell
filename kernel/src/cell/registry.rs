//! Cell Registry and Dependency Graph.

use types::*;
use types::VAddr; // VAddr is in types
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    Unloaded,
    Loading,
    Ready,
    Running,
    Stopped,
    Failed,
}

/// Cell node in the dependency graph.
pub struct CellNode {
    /// Unique identifier.
    pub id: CellId,
    /// Cells this one imports from.
    pub imports: Vec<CellId>,
    /// Cells that import from this one.
    pub exported_to: Vec<CellId>,
    /// Current state.
    pub state: CellState,
    /// Base address in memory.
    pub base_addr: VAddr,
    /// Size in bytes.
    pub size: usize,
}

impl CellNode {
    pub fn new(id: CellId, base_addr: VAddr, size: usize) -> Self {
        Self {
            id,
            imports: Vec::new(),
            exported_to: Vec::new(),
            state: CellState::Loading,
            base_addr,
            size,
        }
    }
}
