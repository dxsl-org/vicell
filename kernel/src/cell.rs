//! Cell Metadata, Registry, and Lifecycle Management.
//!
//! Complies with Agent Manifest "LINH HỒN: Quản lý Metadata, Registry, Dependency".

pub mod metadata;
pub mod registry;

// Re-export core types for convenience
pub use metadata::CellHeader;
pub use registry::{CellNode, CellState};
