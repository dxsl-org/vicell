// SPDX-License-Identifier: MPL-2.0
//! Minimal no_std ACPI table parser for x86_64 kernel.
//!
//! Parses RSDP → XSDT/RSDT → MADT (APIC), MCFG (PCIe ECAM), HPET.
//!
//! Design invariants:
//! - NEVER panics; every parse error → warn log → field keeps its default.
//! - Called before paging, so all table access uses the `phys_to_virt` closure
//!   (typically HHDM offset). Limine maps all RAM at HHDM+phys.
//! - Only x86_64 needs this; other arches use DTB via `platform::init`.

/// Parsed addresses from ACPI tables.
///
/// Every field has a safe hardware-compatible default so boot succeeds even
/// when the RSDP is absent or a table is malformed.
#[derive(Clone, Copy, Debug)]
pub struct AcpiInfo {
    /// Local APIC MMIO base (MADT Local APIC Address field or type-5 override).
    pub lapic_base: u64,
    /// I/O APIC MMIO base (MADT type-1 entry).
    pub ioapic_base: u64,
    /// I/O APIC Global System Interrupt base (MADT type-1 gsi_base).
    pub ioapic_gsi_base: u32,
    /// HPET event timer block address (HPET table GAS address field).
    pub hpet_base: u64,
    /// PCIe ECAM config space base (MCFG allocation[0].base_address).
    pub ecam_base: u64,
    /// ISA IRQ → GSI override table. Index = ISA IRQ (0–15); value = GSI.
    /// Entries not overridden by MADT type-2 keep identity mapping (IRQ N → GSI N).
    pub irq_overrides: [u32; 16],
}

impl Default for AcpiInfo {
    fn default() -> Self {
        let mut overrides = [0u32; 16];
        for (i, v) in overrides.iter_mut().enumerate() {
            *v = i as u32;
        }
        Self {
            lapic_base: 0xFEE0_0000,
            ioapic_base: 0xFEC0_0000,
            ioapic_gsi_base: 0,
            hpet_base: 0xFED0_0000,
            ecam_base: 0xB000_0000,
            irq_overrides: overrides,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal layout types — packed structs used only for read_unaligned casts.
// ---------------------------------------------------------------------------

// NOTE: SdtHeader / MadtHeader / Gas are documented here as layout reference
// but not instantiated — we read individual fields by explicit byte offset
// to avoid references to packed struct fields (which Rust forbids).
// Table offsets are documented in each parser function.

/// RSDP v1 (revision 0) — 20 bytes.
#[derive(Copy, Clone)]
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

/// RSDP v2 (revision ≥ 2) extension — layout after the v1 struct.
/// Total size: 36 bytes.
#[derive(Copy, Clone)]
#[repr(C, packed)]
struct RsdpV2 {
    v1: Rsdp,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    _reserved: [u8; 3],
}

// ---------------------------------------------------------------------------
// Safety helpers
// ---------------------------------------------------------------------------

/// Read a `T` from a raw virtual address without alignment requirement.
///
/// # Safety
/// `virt` must be a valid, readable virtual address pointing to at least
/// `size_of::<T>()` bytes. The value is read byte-by-byte to avoid UB from
/// unaligned packed struct access.
#[inline]
unsafe fn read_unaligned<T: Copy>(virt: usize) -> T {
    // SAFETY: caller guarantees virt is valid.
    unsafe { core::ptr::read_unaligned(virt as *const T) }
}

/// Compute the byte checksum of a memory region (ACPI table validation).
///
/// # Safety
/// `[base, base + len)` must be readable virtual memory.
unsafe fn table_checksum(base: usize, len: usize) -> u8 {
    let mut sum: u8 = 0;
    for i in 0..len {
        // SAFETY: caller guarantees [base, base+len) is readable.
        let byte = unsafe { core::ptr::read_volatile((base + i) as *const u8) };
        sum = sum.wrapping_add(byte);
    }
    sum
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse ACPI tables starting from `rsdp_phys`.
///
/// `phys_to_virt` converts a physical address to a kernel-accessible virtual
/// address (typically `phys + HHDM_OFFSET` under Limine).
///
/// Returns `AcpiInfo` with parsed values; any field that cannot be parsed keeps
/// the QEMU q35 default so the system boots even if ACPI is partially broken.
///
/// # Panics
/// Never — all errors produce a log warning and fall through to defaults.
pub fn parse(rsdp_phys: usize, phys_to_virt: impl Fn(usize) -> usize) -> AcpiInfo {
    let mut info = AcpiInfo::default();

    if rsdp_phys == 0 {
        log::warn!("[acpi] RSDP physical address is null — using defaults");
        return info;
    }

    let rsdp_virt = phys_to_virt(rsdp_phys);

    // --- Validate RSDP signature ---
    // SAFETY: Limine guarantees the RSDP response contains a valid physical
    // address to the RSDP structure, mapped via HHDM. We read cautiously.
    let sig_bytes: [u8; 8] = unsafe { read_unaligned(rsdp_virt) };
    if &sig_bytes != b"RSD PTR " {
        log::warn!("[acpi] RSDP signature mismatch — using defaults");
        return info;
    }

    // --- Validate RSDP v1 checksum (bytes 0..19) ---
    // SAFETY: RSDP is at least 20 bytes (v1 size); Limine guarantees it.
    let cksum = unsafe { table_checksum(rsdp_virt, 20) };
    if cksum != 0 {
        log::warn!("[acpi] RSDP v1 checksum failed ({}) — using defaults", cksum);
        return info;
    }

    // SAFETY: RSDP is at least size_of::<Rsdp>() bytes.
    let rsdp: Rsdp = unsafe { read_unaligned(rsdp_virt) };
    let revision = rsdp.revision;

    if revision >= 2 {
        // XSDT path: 64-bit table pointers.
        // SAFETY: RSDP v2 is at least size_of::<RsdpV2>() bytes; Limine guarantees it.
        let rsdp_v2: RsdpV2 = unsafe { read_unaligned(rsdp_virt) };
        let xsdt_phys = { let a = rsdp_v2.xsdt_address; a } as usize;
        if xsdt_phys == 0 {
            log::warn!("[acpi] XSDT address is null — using defaults");
            return info;
        }
        // SAFETY: xsdt_phys is a physical address from RSDP v2; phys_to_virt
        // maps it to a valid HHDM virtual address readable by the kernel.
        unsafe { parse_xsdt(xsdt_phys, &phys_to_virt, &mut info); }
    } else {
        // RSDT path: 32-bit table pointers.
        let rsdt_phys = { let a = rsdp.rsdt_address; a } as usize;
        if rsdt_phys == 0 {
            log::warn!("[acpi] RSDT address is null — using defaults");
            return info;
        }
        // SAFETY: rsdt_phys is a physical address from RSDP v1; phys_to_virt
        // maps it to a valid HHDM virtual address readable by the kernel.
        unsafe { parse_rsdt(rsdt_phys, &phys_to_virt, &mut info); }
    }

    info
}

// ---------------------------------------------------------------------------
// XSDT / RSDT iteration
// ---------------------------------------------------------------------------

/// Validate an SDT header checksum and return `(virt_base, length)` if valid.
///
/// Returns `None` and logs a warning if the checksum fails.
unsafe fn validate_sdt(phys: usize, phys_to_virt: &impl Fn(usize) -> usize) -> Option<(usize, usize)> {
    let virt = phys_to_virt(phys);
    // Read the length field at offset 4 (4 bytes into the header).
    // SAFETY: phys points to a valid SDT; length field is at offset 4.
    let length = unsafe { core::ptr::read_unaligned((virt + 4) as *const u32) } as usize;
    if length < 36 || length > 0x10_0000 {
        log::warn!("[acpi] SDT at {:#x} has implausible length {} — skipping", phys, length);
        return None;
    }
    // SAFETY: [virt, virt+length) is within a valid Limine HHDM-mapped region.
    let cksum = unsafe { table_checksum(virt, length) };
    if cksum != 0 {
        log::warn!("[acpi] SDT at {:#x} checksum failed ({}) — skipping", phys, cksum);
        return None;
    }
    Some((virt, length))
}

/// Read the 4-byte signature at `virt`.
#[inline]
unsafe fn read_sig(virt: usize) -> [u8; 4] {
    // SAFETY: caller guarantees virt is within a valid SDT.
    unsafe { core::ptr::read_unaligned(virt as *const [u8; 4]) }
}

/// Iterate XSDT (64-bit pointer array) and dispatch each child SDT.
unsafe fn parse_xsdt(phys: usize, p2v: &impl Fn(usize) -> usize, info: &mut AcpiInfo) {
    let Some((virt, length)) = (unsafe { validate_sdt(phys, p2v) }) else {
        log::warn!("[acpi] XSDT validation failed — using defaults");
        return;
    };

    // Entries start at byte 36 (after the 36-byte common header).
    // Each entry is an 8-byte physical pointer.
    let entries_start = virt + 36;
    let entries_len = (length - 36) / 8;

    for i in 0..entries_len {
        // SAFETY: within validated XSDT body.
        let child_phys = unsafe { core::ptr::read_unaligned((entries_start + i * 8) as *const u64) } as usize;
        if child_phys == 0 { continue; }
        dispatch_sdt(child_phys, p2v, info);
    }
}

/// Iterate RSDT (32-bit pointer array) and dispatch each child SDT.
unsafe fn parse_rsdt(phys: usize, p2v: &impl Fn(usize) -> usize, info: &mut AcpiInfo) {
    let Some((virt, length)) = (unsafe { validate_sdt(phys, p2v) }) else {
        log::warn!("[acpi] RSDT validation failed — using defaults");
        return;
    };

    // Entries start at byte 36; each entry is a 4-byte physical pointer.
    let entries_start = virt + 36;
    let entries_len = (length - 36) / 4;

    for i in 0..entries_len {
        // SAFETY: within validated RSDT body.
        let child_phys = unsafe { core::ptr::read_unaligned((entries_start + i * 4) as *const u32) } as usize;
        if child_phys == 0 { continue; }
        dispatch_sdt(child_phys, p2v, info);
    }
}

/// Validate and dispatch a child SDT by its 4-byte signature.
fn dispatch_sdt(phys: usize, p2v: &impl Fn(usize) -> usize, info: &mut AcpiInfo) {
    let Some((virt, length)) = (unsafe { validate_sdt(phys, p2v) }) else { return; };
    // SAFETY: virt is the start of a validated SDT.
    let sig = unsafe { read_sig(virt) };

    match &sig {
        b"APIC" => parse_madt(virt, length, info),
        b"MCFG" => parse_mcfg(virt, length, info),
        b"HPET" => parse_hpet(virt, length, info),
        _ => {
            // Skip unknown/unneeded tables silently.
        }
    }
}

// ---------------------------------------------------------------------------
// MADT parser
// ---------------------------------------------------------------------------

/// Parse MADT (Multiple APIC Description Table) for LAPIC/IOAPIC/IRQ overrides.
///
/// Reads:
///   - MADT header `local_apic_address` (32-bit)
///   - Type 1 (I/O APIC): ioapic_addr, gsi_base
///   - Type 2 (Int Source Override): ISA IRQ → GSI mapping
///   - Type 5 (Local APIC Address Override): 64-bit lapic_addr
fn parse_madt(virt: usize, length: usize, info: &mut AcpiInfo) {
    // MADT-specific header starts after the common 36-byte SDT header.
    // Layout: local_apic_address (u32) at offset 36, flags (u32) at offset 40.
    // Entry records begin at offset 44.
    if length < 44 {
        log::warn!("[acpi] MADT too short ({} bytes)", length);
        return;
    }

    // Read Local APIC 32-bit base from MADT header.
    // SAFETY: virt+36 is within the validated MADT body.
    let lapic_addr_32 = unsafe { core::ptr::read_unaligned((virt + 36) as *const u32) };
    if lapic_addr_32 != 0 {
        info.lapic_base = lapic_addr_32 as u64;
    }

    let mut offset = 44usize; // start of MADT entry records
    let end = virt + length;

    while virt + offset + 2 <= end {
        // Each entry: type (u8), length (u8), then type-specific payload.
        // SAFETY: offset is within the validated MADT.
        let entry_type = unsafe { core::ptr::read_volatile((virt + offset) as *const u8) };
        let entry_len  = unsafe { core::ptr::read_volatile((virt + offset + 1) as *const u8) } as usize;

        if entry_len < 2 || virt + offset + entry_len > end {
            // Malformed entry — stop parsing entries but keep what we got.
            log::warn!("[acpi] MADT entry type={} has bad length={}", entry_type, entry_len);
            break;
        }

        match entry_type {
            // Type 0: Processor Local APIC (length 8) — skip (no SMP enum needed).
            0 => {}

            // Type 1: I/O APIC (length 12).
            //   offset+2: ioapic_id (u8)
            //   offset+3: reserved (u8)
            //   offset+4: ioapic_address (u32)
            //   offset+8: gsi_base (u32)
            1 if entry_len >= 12 => {
                let ioapic_addr = unsafe {
                    core::ptr::read_unaligned((virt + offset + 4) as *const u32)
                };
                let gsi_base = unsafe {
                    core::ptr::read_unaligned((virt + offset + 8) as *const u32)
                };
                if ioapic_addr != 0 {
                    info.ioapic_base = ioapic_addr as u64;
                    info.ioapic_gsi_base = gsi_base;
                }
            }

            // Type 2: Interrupt Source Override (length 10).
            //   offset+2: bus (u8) — always 0 for ISA
            //   offset+3: source (u8) — ISA IRQ number
            //   offset+4: gsi (u32)
            //   offset+8: flags (u16)
            2 if entry_len >= 10 => {
                let bus    = unsafe { core::ptr::read_volatile((virt + offset + 2) as *const u8) };
                let source = unsafe { core::ptr::read_volatile((virt + offset + 3) as *const u8) };
                let gsi    = unsafe { core::ptr::read_unaligned((virt + offset + 4) as *const u32) };
                // Only remap ISA bus (bus==0) IRQs that fit in our table.
                if bus == 0 && (source as usize) < 16 {
                    info.irq_overrides[source as usize] = gsi;
                }
            }

            // Type 4: Non-Maskable Interrupt (NMI) (length 6) — skip.
            4 => {}

            // Type 5: Local APIC Address Override (length 12).
            //   offset+2: reserved (u16)
            //   offset+4: lapic_address (u64)
            5 if entry_len >= 12 => {
                let lapic64 = unsafe {
                    core::ptr::read_unaligned((virt + offset + 4) as *const u64)
                };
                if lapic64 != 0 {
                    info.lapic_base = lapic64;
                }
            }

            // Ignore all other entry types (type 3, 6, 7, 9, 0xA, etc.).
            _ => {}
        }

        offset += entry_len;
    }
}

// ---------------------------------------------------------------------------
// MCFG parser
// ---------------------------------------------------------------------------

/// Parse MCFG (Memory Mapped Configuration Space) table.
///
/// Reads the first allocation entry's `base_address` as the PCIe ECAM base.
/// MCFG body layout (after 36-byte SDT header):
///   - 8 bytes reserved
///   - then N × 16-byte allocation entries:
///       base_address (u64), segment (u16), bus_start (u8), bus_end (u8), _reserved (u32)
fn parse_mcfg(virt: usize, length: usize, info: &mut AcpiInfo) {
    // First allocation entry starts at offset 44 (36-byte header + 8-byte reserved).
    if length < 44 + 16 {
        log::warn!("[acpi] MCFG too short ({} bytes) for any allocation entry", length);
        return;
    }

    // SAFETY: offset 44 is within the validated MCFG body.
    let base_addr = unsafe { core::ptr::read_unaligned((virt + 44) as *const u64) };
    if base_addr != 0 {
        info.ecam_base = base_addr;
        log::info!("[acpi] MCFG: PCIe ECAM base = {:#x}", base_addr);
    } else {
        log::warn!("[acpi] MCFG: first allocation base_address is 0 — keeping default");
    }
}

// ---------------------------------------------------------------------------
// HPET parser
// ---------------------------------------------------------------------------

/// Parse HPET table for event timer block MMIO address.
///
/// HPET table layout (after 36-byte SDT header):
///   offset 36: event_timer_block_id (u32) — hardware rev, comparators, etc.
///   offset 40: base_address (GAS, 12 bytes)
///       GAS[0]:  address_space_id (u8) — 0 = MMIO
///       GAS[4..12]: address (u64)
///   offset 52: hpet_number (u8)
///   offset 53: minimum_tick (u16)
///   offset 55: page_protection (u8)
fn parse_hpet(virt: usize, length: usize, info: &mut AcpiInfo) {
    // GAS starts at offset 40; address field is at GAS offset 4 → table offset 44.
    if length < 56 {
        log::warn!("[acpi] HPET table too short ({} bytes)", length);
        return;
    }

    // GAS address_space_id at offset 40.
    // SAFETY: offset 40 is within the validated HPET body.
    let addr_space = unsafe { core::ptr::read_volatile((virt + 40) as *const u8) };
    if addr_space != 0 {
        // Non-zero address space means I/O ports or PCI config — not simple MMIO.
        log::warn!("[acpi] HPET GAS address_space_id={} (not MMIO) — keeping default", addr_space);
        return;
    }

    // GAS address at GAS offset +4 → table offset 44.
    // SAFETY: offset 44 is within the validated HPET body.
    let hpet_addr = unsafe { core::ptr::read_unaligned((virt + 44) as *const u64) };
    if hpet_addr != 0 {
        info.hpet_base = hpet_addr;
        log::info!("[acpi] HPET: event timer block = {:#x}", hpet_addr);
    } else {
        log::warn!("[acpi] HPET: GAS address is 0 — keeping default");
    }
}
