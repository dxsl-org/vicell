//! ELF relocation engine for ViOS cells.
//!
//! Supports the relocation types emitted by the RISC-V LLVM/GCC toolchain
//! when building position-independent cells (`-pie` or `-shared`).

use types::{VAddr, ViError, ViResult};

/// Maximum number of relocations to process per cell.
/// Bounds parsing time; a legitimate cell has at most tens of thousands.
const MAX_RELA_ENTRIES: usize = 65_536;

/// RISC-V ELF relocation types (r_type field in Elf64_Rela.r_info).
#[allow(dead_code)] // reason: table documents all types; only R_RISCV_RELATIVE is used today
mod riscv_reloc_type {
    pub const R_RISCV_NONE: u32 = 0;
    pub const R_RISCV_RELATIVE: u32 = 3;
    pub const R_RISCV_64: u32 = 2;
    pub const R_RISCV_JUMP_SLOT: u32 = 5;
}

/// A single 64-bit RELA entry (Elf64_Rela layout, little-endian).
#[repr(C)]
#[derive(Copy, Clone)]
struct Rela64 {
    /// Offset within the ELF's virtual address space to patch.
    offset: u64,
    /// Relocation type + symbol index packed as (sym<<32 | type).
    info: u64,
    /// Addend to use in the relocation formula.
    addend: i64,
}

impl Rela64 {
    fn r_type(self) -> u32 {
        (self.info & 0xFFFF_FFFF) as u32
    }
}

/// Apply all relocations in the `.rela.dyn` section of an already-mapped ELF.
///
/// `base` is the load base chosen by the kernel.  For cells compiled at a fixed
/// VA (non-PIE linker script), `base == 0` and this function is a no-op beyond
/// bounds checks.  For PIE cells (base-address randomisation or known-base
/// placement), this patches every `R_RISCV_RELATIVE` entry.
///
/// # Errors
/// Returns `ViError::InvalidInput` on malformed or oversized relocation tables.
/// Returns `ViError::NotSupported` if an unsupported relocation type is found.
pub fn apply_relocations(base: VAddr, rela_section: &[u8]) -> ViResult<()> {
    let entry_size = core::mem::size_of::<Rela64>();
    if rela_section.len() % entry_size != 0 {
        log::error!(
            "[reloc] .rela.dyn size {} not a multiple of Rela64 size {}",
            rela_section.len(),
            entry_size
        );
        return Err(ViError::InvalidInput);
    }

    let count = rela_section.len() / entry_size;
    if count > MAX_RELA_ENTRIES {
        log::error!("[reloc] too many relocations: {} > {}", count, MAX_RELA_ENTRIES);
        return Err(ViError::InvalidInput);
    }

    for i in 0..count {
        let offset = i * entry_size;
        // SAFETY: bounds checked (offset + entry_size <= len).  Use
        // read_unaligned because the ELF buffer may not be 8-byte aligned —
        // the backing Vec is u8-aligned; section data inherits that.
        let entry: Rela64 = unsafe {
            core::ptr::read_unaligned(rela_section.as_ptr().add(offset) as *const Rela64)
        };

        match entry.r_type() {
            riscv_reloc_type::R_RISCV_NONE => {}
            riscv_reloc_type::R_RISCV_RELATIVE => {
                // Formula: *ptr = base + addend
                let patch_va = base.wrapping_add(entry.offset as usize);
                let value = base.wrapping_add(entry.addend as usize);
                // SAFETY: patch_va is within the cell's mapped pages; SUM=1
                // allows S-mode to write to U-mode pages.
                unsafe { (patch_va as *mut usize).write_unaligned(value) };
                log::trace!("[reloc] R_RELATIVE @ 0x{:X} = 0x{:X}", patch_va, value);
            }
            riscv_reloc_type::R_RISCV_64 => {
                // R_RISCV_64 with sym_index == 0 is an absolute-address
                // fixup; treat identically to R_RISCV_RELATIVE.
                let sym_index = (entry.info >> 32) as u32;
                if sym_index != 0 {
                    log::error!("[reloc] R_RISCV_64 with non-zero sym {} not supported", sym_index);
                    return Err(ViError::NotSupported);
                }
                let patch_va = base.wrapping_add(entry.offset as usize);
                let value = base.wrapping_add(entry.addend as usize);
                // SAFETY: same invariant as R_RISCV_RELATIVE above.
                unsafe { (patch_va as *mut usize).write_unaligned(value) };
                log::trace!("[reloc] R_RISCV_64 @ 0x{:X} = 0x{:X}", patch_va, value);
            }
            other => {
                log::error!("[reloc] unsupported relocation type {}", other);
                return Err(ViError::NotSupported);
            }
        }
    }

    log::debug!("[reloc] applied {} relocation(s) with base 0x{:X}", count, base);
    Ok(())
}
