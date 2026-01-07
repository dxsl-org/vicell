//! Cell Metadata definitions.

use types::SemVer;

/// Cell metadata header (embedded in `.cell_info` section).
#[repr(C)]
pub struct CellHeader {
    /// Cell name.
    pub name: &'static str,
    /// Cell version.
    pub version: SemVer,
    /// Dependencies (name, version requirement).
    pub deps: &'static [(&'static str, &'static str)],
}
