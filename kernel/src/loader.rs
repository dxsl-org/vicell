//! Cell loader — ELF parsing, relocation, and path-based spawning.

use types::*;

pub mod disk_layout;
pub mod early;
pub mod elf;
pub mod elf_tests;
pub mod reloc;
pub use elf::ElfLoader;

/// ELF parser trait.
pub trait ElfParser {
    /// Parse ELF header, returning entry point and section-header offset.
    fn parse_header(&self, data: &[u8]) -> ViResult<ElfHeader>;

    /// Return the raw bytes of a named section, or `ViError::NotFound`.
    fn get_section<'a>(&self, data: &'a [u8], name: &str) -> ViResult<&'a [u8]>;
}

/// Parsed ELF header fields needed by the spawner.
pub struct ElfHeader {
    /// Entry point virtual address.
    pub entry: VAddr,
    /// Section header table file offset (used for relocation lookups).
    pub shoff: usize,
}

/// Spawn a cell by reading its ELF from a filesystem path.
///
/// Resolution order:
/// 1. If the early-boot cell table has been probed (via `early::EarlyLoader::probe`),
///    reads the ELF directly from the block device at the known LBA.
/// 2. Otherwise returns `ViError::NotFound` — the caller must ensure the early
///    table is probed before calling `spawn_from_path` during bootstrapping.
///
/// After the ELF is loaded into memory, relocations are applied and the cell is
/// enqueued via `crate::task::spawn_from_mem`.
///
/// # Errors
/// - `ViError::NotFound` — path absent from the bootstrap table.
/// - `ViError::InvalidInput` — malformed ELF or unsupported relocation.
/// - `ViError::OutOfMemory` — cannot allocate frames for segments.
pub fn spawn_from_path(path: &str) -> ViResult<usize> {
    // Validate path: must be non-empty, start with '/', length bounded.
    if path.is_empty() || !path.starts_with('/') || path.len() > disk_layout::MAX_CELL_PATH {
        log::error!("[loader] invalid path {:?}", path);
        return Err(ViError::InvalidInput);
    }

    log::info!("[loader] SpawnFromPath: {}", path);

    // Read ELF bytes from the early bootstrap table.
    let elf_bytes = early::EarlyLoader::read_file(path)?;

    // Apply relocations (base = 0 for fixed-VA cells; non-zero for PIE cells).
    // For cells with no .rela.dyn section, get_section returns NotFound — skip.
    let base: VAddr = 0; // fixed-VA cells compiled with shell.ld; PIE support is future work
    let elf_loader = ElfLoader;
    if let Ok(rela_section) = elf_loader.get_section(&elf_bytes, ".rela.dyn") {
        reloc::apply_relocations(base, rela_section)?;
    }

    // Extract cell name from the last path component (e.g. "/bin/shell" → "shell").
    let name = path.rsplit('/').next().unwrap_or(path);

    // Spawn via the existing in-memory spawn path (ELF parse + segment map).
    let tid = crate::task::spawn_from_mem(&elf_bytes, name, CellId(0), alloc::vec::Vec::new())
        .map_err(|_| ViError::OutOfMemory)?;

    // Grant raw block-I/O to the VFS service only. Boot-order-independent
    // replacement for the former `VFS_TASK_ID == 3` hardcode (Phase G).
    // Phase H: replace with a formal CapPerms::BLOCK_IO capability token.
    if path.ends_with("/bin/vfs") {
        if let Some(sched) = crate::task::SCHEDULER.lock().as_mut() {
            if let Some(task) = sched.tasks.get_mut(&tid) {
                task.can_block_io = true;
            }
        }
    }
    Ok(tid)
}

/// Linker trait (reserved for future dynamic-linking support).
#[allow(dead_code)] // reason: trait body used by future Cell hot-swap (Phase 20)
pub trait Linker {
    fn load_cell(&mut self, data: &[u8]) -> ViResult<CellId>;
    fn resolve_symbol(&self, name: &str) -> ViResult<VAddr>;
    fn unload_cell(&mut self, id: CellId) -> ViResult<()>;
}
