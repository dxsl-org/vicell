//! Task scheduling primitives exposed in the stable Cell API.

/// Priority tier for spawned Cells.
///
/// Higher variant value = higher scheduling priority.  Stored as `u8` in
/// the Task Control Block so it fits in the same word as other flags.
///
/// # Ordering
/// `RealTime > Normal > Background`.  The scheduler always runs the
/// highest-priority ready Cell; ties are broken by FIFO within the same
/// tier.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Lowest priority — batch workloads, AI inference, non-urgent logging.
    Background = 0,
    /// Default priority — shell, VFS, config, network.
    Normal = 1,
    /// Highest priority — robot control, sensor polling, hard-deadline tasks.
    RealTime = 2,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}
