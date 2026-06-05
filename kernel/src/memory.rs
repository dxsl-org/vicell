//! Memory management interfaces.

use crate::*;

pub mod frame;
pub mod heap;
pub mod paging;
pub mod rt_heap;
pub mod tests;

/// Ownership registry entry.
pub struct AllocationInfo {
    /// Address of allocation.
    pub address: VAddr,
    /// Size in bytes.
    pub size: usize,
    /// Owning Cell ID.
    pub owner: CellId,
}

/// Global memory management trait (to be implemented).
pub trait ViGlobalMemoryManager {
    /// Allocate memory for a Cell.
    fn alloc(&self, size: usize, owner: CellId) -> ViResult<VAddr>;

    /// Free memory owned by a Cell.
    fn free(&self, addr: VAddr) -> ViResult<()>;

    /// Transfer ownership of an allocation.
    fn transfer_ownership(&self, addr: VAddr, new_owner: CellId) -> ViResult<()>;
}
