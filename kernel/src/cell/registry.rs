//! Cell Registry Implementation
//! Manages the lifecycle and metadata of all loaded Cells in the system.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use crate::sync::Spinlock;
use types::{CellId, ViResult, ViError};
use super::CellNode;

/// Global Cell Registry
pub static CELL_REGISTRY: Spinlock<CellRegistry> = Spinlock::new(CellRegistry::new());

pub struct CellRegistry {
    cells: BTreeMap<CellId, Arc<CellNode>>,
    name_index: BTreeMap<alloc::string::String, CellId>,
    next_id: usize,
}

impl CellRegistry {
    pub const fn new() -> Self {
        Self {
            cells: BTreeMap::new(),
            name_index: BTreeMap::new(),
            next_id: 1, // 0 is reserved/kernel?
        }
    }

    pub fn register(&mut self, mut node: CellNode) -> ViResult<CellId> {
        // Assign ID if not already set (assuming 0 means auto-assign)
        if node.id.0 == 0 {
            node.id = CellId(self.next_id as u64);
            self.next_id += 1;
        }

        if self.cells.contains_key(&node.id) {
            return Err(ViError::AlreadyExists);
        }

        let id = node.id;
        let name = node.name.clone();

        let node_arc = Arc::new(node);
        self.cells.insert(id, node_arc);
        self.name_index.insert(name, id);

        Ok(id)
    }

    pub fn get(&self, id: CellId) -> Option<Arc<CellNode>> {
        self.cells.get(&id).cloned()
    }

    pub fn get_by_name(&self, name: &str) -> Option<Arc<CellNode>> {
        self.name_index.get(name).and_then(|id| self.cells.get(id).cloned())
    }
}
