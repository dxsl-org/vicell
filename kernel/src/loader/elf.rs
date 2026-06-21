//! ELF Parsing Logic
use super::{ElfHeader, ElfParser};
use types::*;
use xmas_elf::ElfFile;

/// Maximum user-space virtual address for RISC-V SV39.
///
/// SV39 splits the 39-bit VA space into:
///   0x0000_0000_0000 – 0x003F_FFFF_FFFF  (user / lower half, 256 GB)
///   0xFFC0_0000_0000 – 0xFFFF_FFFF_FFFF  (kernel / upper half, 256 GB)
///
/// 0x8000_0000 was wrong — it only allowed 2 GB and coincidentally matched
/// the RISC-V physical RAM base (0x8000_0000 on QEMU virt), causing every
/// cell ELF compiled at 0x8800_0000+ to be rejected.
///
/// The real boundary is half the SV39 address space: 2^38 = 0x40_0000_0000.
/// Cells compiled at 0x0040_0000 (4 MB) are safely within this range.
// SV39 user-half upper bound (256 GB). On riscv32 the address space is 32-bit
// so this value is clamped to usize::MAX (0xFFFF_FFFF), which is still correct
// as a "no address may be >= 4 GB" guard for riscv32 cells.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
const USER_VADDR_MAX: usize = 0x40_0000_0000; // 256 GB — SV39 user half
#[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
const USER_VADDR_MAX: usize = 0xFFFF_FFFF; // 4 GB — full 32-bit address space

pub struct ElfLoader;

impl ElfLoader {
    /// Load loadable segments into memory.
    ///
    /// `load_base` is added to every segment's `p_vaddr` before mapping:
    /// - For fixed-VA cells (ET_EXEC, non-PIE): pass `0` — uses p_vaddr directly.
    /// - For PIE cells (ET_DYN): pass the VA base allocated by `va_alloc::alloc_cell_va`.
    ///
    /// Returns the list of mapped (va, frame) pairs so `CellSegments` can reclaim
    /// them when the cell dies.
    pub fn load_segments(
        &self,
        data: &[u8],
        frame_allocator: &mut crate::memory::frame::FrameAllocator,
        load_base: usize,
    ) -> ViResult<alloc::vec::Vec<(VAddr, PhysAddr)>> {
        let elf = ElfFile::new(data).map_err(|_| ViError::InvalidInput)?;
        // Record each mapped (vaddr, frame) so the cell's segment frames can be
        // reclaimed when it dies (see task::stack::CellSegments) — otherwise they leak.
        use crate::memory::paging::Flags;
        let mut mapped: alloc::vec::Vec<(VAddr, PhysAddr, Flags)> = alloc::vec::Vec::new();

        for ph in elf.program_iter() {
            if let Ok(xmas_elf::program::Type::Load) = ph.get_type() {
                let file_offset = ph.offset() as usize;
                // For PIE cells load_base relocates every segment; for fixed-VA
                // cells load_base == 0 so p_vaddr is used verbatim.
                let vaddr = (ph.virtual_addr() as usize).wrapping_add(load_base);
                let mem_size = ph.mem_size() as usize;
                let file_size = ph.file_size() as usize;
                let ph_flags = ph.flags();

                // --- Header sanity checks ---
                // file_size MUST NOT exceed mem_size (the rest is BSS).
                if file_size > mem_size {
                    log::error!(
                        "ELF: rejecting segment with file_size={} > mem_size={}",
                        file_size,
                        mem_size
                    );
                    return Err(ViError::InvalidInput);
                }
                // file_offset + file_size must fit inside the ELF buffer.
                let file_end = file_offset.checked_add(file_size).ok_or(ViError::InvalidInput)?;
                if file_end > data.len() {
                    log::error!(
                        "ELF: segment file range {}..{} exceeds buffer len {}",
                        file_offset,
                        file_end,
                        data.len()
                    );
                    return Err(ViError::InvalidInput);
                }
                // vaddr + mem_size must not overflow and must lie below the
                // kernel VA window — prevents user ELF clobbering kernel maps.
                let end_addr = vaddr.checked_add(mem_size).ok_or(ViError::InvalidInput)?;
                if vaddr >= USER_VADDR_MAX || end_addr > USER_VADDR_MAX {
                    log::error!(
                        "ELF: segment VA range 0x{:X}-0x{:X} outside user space",
                        vaddr,
                        end_addr
                    );
                    return Err(ViError::PermissionDenied);
                }

                let start_addr = vaddr;

                // Align start/end to page boundaries
                let start_page = start_addr & !(4096 - 1);
                let end_page = end_addr.checked_add(4095).ok_or(ViError::InvalidInput)? & !(4096 - 1);

                // --- Translate ELF p_flags to page-table flags ---
                // p_flags bits: 0x1=X, 0x2=W, 0x4=R. Default deny if all zero.
                use crate::memory::paging::Flags;
                let mut perm_bits = Flags::VALID | Flags::USER | Flags::ACCESSED;
                if ph_flags.is_read() {
                    perm_bits = perm_bits | Flags::READ;
                }
                if ph_flags.is_write() {
                    perm_bits = perm_bits | Flags::WRITE | Flags::DIRTY;
                }
                if ph_flags.is_execute() {
                    perm_bits = perm_bits | Flags::EXECUTE;
                }
                let flags = Flags::from_bits(perm_bits);

                // Map pages
                let mut current_page = start_page;
                while current_page < end_page {
                    // Overwrite guard: reject VA collision with kernel MMIO or a
                    // *different* live cell. Allow shared pages from *this* cell's own
                    // adjacent PT_LOAD segments (already_ours == true).
                    //
                    // PIE linker scripts may place adjacent sections (e.g. .text R-X
                    // and .rodata R--) at non-page-aligned offsets so the two segments
                    // share the same physical page.  In that case already_ours==true and
                    // we reuse the frame, merging the new segment's flags into the page
                    // rather than allocating a fresh one or blindly overwriting flags.
                    let already_ours = mapped.iter().any(|(va, _, _)| *va == current_page);
                    if !already_ours && crate::memory::paging::virt_to_phys(current_page).is_some() {
                        log::error!(
                            "ELF: load VA 0x{:X} already mapped — rejecting spawn (VA collision with a live cell or kernel MMIO; fix the cell's linker script)",
                            current_page
                        );
                        for &(va, fr, _) in &mapped {
                            let _ = crate::memory::paging::unmap_page(va);
                            frame_allocator.deallocate_frame(fr);
                        }
                        return Err(ViError::PermissionDenied);
                    }

                    // Get the backing frame's kernel-accessible VA for copying.
                    let frame_virt = if already_ours {
                        // Shared page from an earlier LOAD segment (e.g. .rodata R-- and
                        // .data/RELRO RW- sharing a boundary page).  Reuse the existing
                        // frame but OR the new segment's permission bits in so the page
                        // satisfies both segments' access requirements.
                        let phys = crate::memory::paging::virt_to_phys(current_page)
                            .expect("already_ours but virt_to_phys returned None");
                        if let Some(entry) = mapped.iter_mut().find(|(va, _, _)| *va == current_page) {
                            let merged = Flags::from_bits(entry.2.bits() | flags.bits());
                            if merged != entry.2 {
                                entry.2 = merged;
                                let _ = crate::memory::paging::map_page(frame_allocator, current_page, phys, merged);
                            }
                        }
                        crate::memory::frame::phys_to_virt(phys)
                    } else {
                        let buf_frame = frame_allocator
                            .allocate_frame()
                            .ok_or(ViError::OutOfMemory)?;

                        crate::memory::paging::map_page(
                            frame_allocator,
                            current_page,
                            buf_frame,
                            flags,
                        )
                        .map_err(|_| ViError::OutOfMemory)?;

                        // Track for reclamation on cell death.
                        mapped.push((current_page, buf_frame, flags));

                        // Zero the frame first (simplifies BSS and padding, and
                        // prevents info-leak from previous frame owner).
                        // Use phys_to_virt: on RISC-V it's a no-op (identity map);
                        // on x86_64 physical RAM is only accessible via HHDM_BASE+phys.
                        let fv = crate::memory::frame::phys_to_virt(buf_frame);
                        unsafe { core::ptr::write_bytes(fv as *mut u8, 0, 4096); }
                        fv
                    };

                    // Intersection of [page, page+4096) AND [vaddr, vaddr+file_size)
                    let page_start_vaz = current_page;
                    let page_end_vaz = current_page + 4096;
                    let copy_start_v = core::cmp::max(page_start_vaz, vaddr);
                    let copy_end_v = core::cmp::min(page_end_vaz, vaddr + file_size);

                    if copy_start_v < copy_end_v {
                        let len = copy_end_v - copy_start_v;
                        let dst_offset = copy_start_v - page_start_vaz;
                        let src_offset_in_file = file_offset + (copy_start_v - vaddr);
                        // file_end was already validated above; this guards
                        // arithmetic on `len` from any rounding surprise.
                        let src_end = src_offset_in_file
                            .checked_add(len)
                            .ok_or(ViError::InvalidInput)?;
                        if src_end <= data.len() {
                            let src = &data[src_offset_in_file..src_end];
                            unsafe {
                                let dst = (frame_virt as *mut u8).add(dst_offset);
                                core::ptr::copy_nonoverlapping(src.as_ptr(), dst, len);
                            }
                        }
                    }

                    current_page += 4096;
                }

                log::info!(
                    "ELF LOAD: 0x{:X}-0x{:X} flags={}{}{}",
                    start_addr,
                    end_addr,
                    if ph_flags.is_read() { 'R' } else { '-' },
                    if ph_flags.is_write() { 'W' } else { '-' },
                    if ph_flags.is_execute() { 'X' } else { '-' },
                );
            }
        }
        Ok(mapped.into_iter().map(|(va, phys, _)| (va, phys)).collect())
    }
}

impl ElfParser for ElfLoader {
    fn parse_header(&self, data: &[u8]) -> ViResult<ElfHeader> {
        let elf = ElfFile::new(data).map_err(|_| ViError::InvalidInput)?;

        // Verify architecture (RISC-V 64)
        // Header check is implicit in successful new(), but specific machine check?
        // elf.header.pt2.machine() == xmas_elf::header::Machine::RISC_V

        Ok(ElfHeader {
            entry: elf.header.pt2.entry_point() as usize,
            shoff: elf.header.pt2.sh_offset() as usize,
        })
    }

    fn get_section<'a>(&self, data: &'a [u8], name: &str) -> ViResult<&'a [u8]> {
        let elf = ElfFile::new(data).map_err(|_| ViError::InvalidInput)?;
        match elf.find_section_by_name(name) {
            Some(section) => Ok(section.raw_data(&elf)),
            None => Err(ViError::NotFound),
        }
    }
}
