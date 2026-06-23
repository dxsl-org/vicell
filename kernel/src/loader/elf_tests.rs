//! Boot-time tests for the ELF loader and relocation engine.
//!
//! Functions are `pub` so `kernel/src/loader.rs` can invoke them from the
//! boot-time test runner.  Every function logs with `log::info!` and asserts
//! with standard `assert!`/`assert_eq!` — a failing assert panics the kernel,
//! which is intentional (hard failure = detected early).

use api::manifest::{
    CellManifest, MANIFEST_FLAG_BLOCK_IO, MANIFEST_FLAG_NETWORK, MANIFEST_FLAGS_MASK,
    MANIFEST_MAGIC, MANIFEST_VERSION,
};
use api::syscall::ViSyscall;
use types::{ViError, ViResult};

/// Run all ELF + relocation tests and log a summary.
pub fn run_all() {
    log::info!("=== ELF Loader Tests ===");
    test_spawn_path_empty_rejected();
    test_spawn_path_no_leading_slash_rejected();
    test_spawn_path_too_long_rejected();
    test_spawn_path_valid_format_accepted();
    test_reloc_empty_section_ok();
    test_reloc_non_multiple_size_rejected();
    test_reloc_too_many_entries_rejected();
    test_reloc_unsupported_type_rejected();
    test_reloc_none_type_noop();
    test_reloc_relative_patches_memory();
    test_manifest_size_is_8();
    test_manifest_parsing_valid();
    test_manifest_parsing_bad_magic();
    test_manifest_parsing_short();
    test_manifest_parsing_bad_version();
    test_manifest_reserved_flags_rejected();
    test_manifest_network_false_grants_no_network_cap();
    test_force_exit_opcode_mapped();
    test_force_exit_allowlist_bit_none();
    // Cell signing tests.
    test_signing_self_test_passes();
    test_signing_extract_sig_none_for_empty_slice();
    test_signing_extract_sig_none_for_non_elf();
    test_signing_extract_sig_some_from_constructed_elf();
    test_signing_required_flag_off_in_dev_build();
    log::info!("=== ELF Loader Tests PASSED ===");
}

// ─── spawn_from_path path validation ─────────────────────────────────────────

fn expect_invalid(res: ViResult<usize>, label: &str) {
    match res {
        Err(ViError::InvalidInput) | Err(ViError::NotFound) => {}
        other => panic!("expected error for {}, got {:?}", label, other),
    }
}

fn test_spawn_path_empty_rejected() {
    let res = crate::loader::spawn_from_path("", crate::task::cap::Spawner::Root);
    expect_invalid(res, "empty path");
    log::info!("  [ok] empty path rejected");
}

fn test_spawn_path_no_leading_slash_rejected() {
    let res = crate::loader::spawn_from_path("bin/shell", crate::task::cap::Spawner::Root);
    expect_invalid(res, "no leading slash");
    log::info!("  [ok] path without leading '/' rejected");
}

fn test_spawn_path_too_long_rejected() {
    let long: alloc::string::String = "/".repeat(300);
    let res = crate::loader::spawn_from_path(&long, crate::task::cap::Spawner::Root);
    expect_invalid(res, "path too long");
    log::info!("  [ok] path longer than MAX_CELL_PATH rejected");
}

fn test_spawn_path_valid_format_accepted() {
    // A well-formatted path may still fail with NotFound (disk not ready) —
    // that is acceptable; only InvalidInput counts as a format rejection.
    let res = crate::loader::spawn_from_path("/bin/nonexistent-elf-for-test", crate::task::cap::Spawner::Root);
    match res {
        Err(ViError::NotFound) | Ok(_) => {}
        Err(ViError::InvalidInput) => panic!("well-formed path should not be rejected as InvalidInput"),
        Err(e) => {
            log::warn!("  [ok] /bin/nonexistent-elf-for-test → {:?} (expected NotFound)", e);
        }
    }
    log::info!("  [ok] well-formed path passes format validation");
}

// ─── apply_relocations ───────────────────────────────────────────────────────

/// Construct the 24-byte raw encoding of a single Rela64 entry.
/// Layout (LE): offset:u64, info:u64, addend:i64
fn make_rela(offset: u64, r_type: u32, addend: i64) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..8].copy_from_slice(&offset.to_le_bytes());
    b[8..16].copy_from_slice(&(r_type as u64).to_le_bytes()); // sym=0 in high 32 bits
    b[16..24].copy_from_slice(&(addend as u64).to_le_bytes());
    b
}

fn test_reloc_empty_section_ok() {
    let res = crate::loader::reloc::apply_relocations(0, &[]);
    assert!(res.is_ok(), "empty section should succeed: {:?}", res);
    log::info!("  [ok] empty .rela.dyn → Ok");
}

fn test_reloc_non_multiple_size_rejected() {
    // 7 bytes is not a multiple of 24 (sizeof Rela64).
    let bad = [0u8; 7];
    let res = crate::loader::reloc::apply_relocations(0, &bad);
    assert_eq!(res, Err(ViError::InvalidInput), "non-multiple size should be InvalidInput");
    log::info!("  [ok] non-multiple rela size → InvalidInput");
}

fn test_reloc_too_many_entries_rejected() {
    // 65_537 * 24 bytes > MAX_RELA_ENTRIES limit.
    const OVER_LIMIT: usize = 65_537;
    let big = alloc::vec![0u8; OVER_LIMIT * 24];
    let res = crate::loader::reloc::apply_relocations(0, &big);
    assert_eq!(res, Err(ViError::InvalidInput), "over-limit count should be InvalidInput");
    log::info!("  [ok] {} entries (> 65536) → InvalidInput", OVER_LIMIT);
}

fn test_reloc_unsupported_type_rejected() {
    // Type 0xFF is not a recognised RISC-V relocation.
    let entry = make_rela(0, 0xFF, 0);
    let res = crate::loader::reloc::apply_relocations(0, &entry);
    assert_eq!(res, Err(ViError::NotSupported), "unknown type should be NotSupported");
    log::info!("  [ok] unknown reloc type 0xFF → NotSupported");
}

fn test_reloc_none_type_noop() {
    // R_RISCV_NONE (type=0) must be silently skipped.
    let entry = make_rela(0, 0, 0); // type=0
    let res = crate::loader::reloc::apply_relocations(0, &entry);
    assert!(res.is_ok(), "R_RISCV_NONE should be a no-op: {:?}", res);
    log::info!("  [ok] R_RISCV_NONE → no-op, Ok");
}

fn test_reloc_relative_patches_memory() {
    // Allocate a writable buffer; use its address as `base`.
    // Create R_RISCV_RELATIVE (type=3): offset=0, addend=0x400.
    // Expected result: *(base + 0) = base + 0x400.
    let mut buf = alloc::vec![0u8; 64];
    let base = buf.as_mut_ptr() as usize;

    let entry = make_rela(0, 3, 0x400_i64); // R_RISCV_RELATIVE
    let res = crate::loader::reloc::apply_relocations(base, &entry);
    assert!(res.is_ok(), "R_RISCV_RELATIVE should succeed: {:?}", res);

    // Read back the patched value (usize-width, unaligned-safe).
    // SAFETY: buf is alive for the duration of this test; we wrote exactly
    // sizeof(usize) bytes at offset 0 via apply_relocations.
    let patched: usize = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const usize)
    };
    let expected = base.wrapping_add(0x400);
    assert_eq!(patched, expected, "R_RISCV_RELATIVE patch value mismatch");
    log::info!("  [ok] R_RISCV_RELATIVE patched 0x{:X} → 0x{:X}", base, expected);
}

// ─── CellManifest parsing ────────────────────────────────────────────────────

fn manifest_bytes(magic: u32, version: u8, flags: u8) -> [u8; 8] {
    let m = magic.to_le_bytes();
    [m[0], m[1], m[2], m[3], version, flags, 0, 0]
}

fn test_manifest_size_is_8() {
    assert_eq!(
        core::mem::size_of::<CellManifest>(), 8,
        "CellManifest must be exactly 8 bytes (ABI invariant)"
    );
    log::info!("  [ok] CellManifest is 8 bytes");
}

fn test_manifest_parsing_valid() {
    let bytes = manifest_bytes(
        MANIFEST_MAGIC, MANIFEST_VERSION,
        MANIFEST_FLAG_BLOCK_IO | MANIFEST_FLAG_NETWORK,
    );
    let m = CellManifest::from_bytes(&bytes).expect("valid manifest must parse");
    assert!(m.has_block_io(), "block_io flag must be set");
    assert!(m.has_network(),  "network flag must be set");
    assert!(!m.has_spawn(),   "spawn flag must be clear");
    assert!(m.declares_any_privilege(), "declares_any_privilege must be true");
    log::info!("  [ok] valid manifest parses with correct flags");
}

fn test_manifest_parsing_bad_magic() {
    let bytes = manifest_bytes(0xDEAD_BEEF, MANIFEST_VERSION, 0);
    assert!(
        CellManifest::from_bytes(&bytes).is_none(),
        "wrong magic must return None"
    );
    log::info!("  [ok] bad magic rejected");
}

fn test_manifest_parsing_short() {
    assert!(
        CellManifest::from_bytes(&[0u8; 4]).is_none(),
        "slice shorter than 8 bytes must return None"
    );
    log::info!("  [ok] short slice rejected");
}

fn test_manifest_parsing_bad_version() {
    let bytes = manifest_bytes(MANIFEST_MAGIC, MANIFEST_VERSION.wrapping_add(1), 0);
    assert!(
        CellManifest::from_bytes(&bytes).is_none(),
        "unsupported version must return None"
    );
    log::info!("  [ok] bad version rejected");
}

fn test_manifest_reserved_flags_rejected() {
    // Any bit above the defined mask must be rejected — prevents stale v1 binaries
    // from silently gaining a future-version capability via a reserved bit.
    let reserved = !MANIFEST_FLAGS_MASK; // e.g., 0xF8
    let bytes = manifest_bytes(MANIFEST_MAGIC, MANIFEST_VERSION, reserved | 0x01);
    assert!(
        CellManifest::from_bytes(&bytes).is_none(),
        "reserved flags must return None"
    );
    // Pure reserved (no defined flags): also rejected.
    let bytes2 = manifest_bytes(MANIFEST_MAGIC, MANIFEST_VERSION, 0x08);
    assert!(
        CellManifest::from_bytes(&bytes2).is_none(),
        "reserved-only flags must return None"
    );
    log::info!("  [ok] reserved flag bits rejected");
}

fn test_force_exit_opcode_mapped() {
    // Opcode 61 must resolve to ForceExit; any other result means the dispatcher
    // would silently ignore ForceExit calls.
    assert!(matches!(ViSyscall::from(61), ViSyscall::ForceExit),
        "opcode 61 must resolve to ViSyscall::ForceExit");
    log::info!("  [ok] opcode 61 → ForceExit");
}

fn test_force_exit_allowlist_bit_none() {
    // ForceExit must bypass the allowlist (like Exit/Yield); SpawnCap is the gate.
    assert!(ViSyscall::ForceExit.allowlist_bit().is_none(),
        "ForceExit must not have an allowlist bit — SpawnCap is the authority check");
    log::info!("  [ok] ForceExit allowlist_bit = None");
}

fn test_manifest_network_false_grants_no_network_cap() {
    let m = CellManifest::new(true, false, false, false, false, false);
    assert!(m.has_block_io(),   "block_io=true must set block_io flag");
    assert!(!m.has_network(),   "network=false must NOT set network flag");
    assert!(!m.has_spawn(),     "spawn=false must NOT set spawn flag");
    assert!(m.declares_any_privilege(), "block_io alone is still a privilege");
    log::info!("  [ok] network=false → no NetworkCap granted");
}

// ─── Cell signing tests ───────────────────────────────────────────────────────

fn test_signing_self_test_passes() {
    assert!(crate::signing::self_test(), "signing::self_test must pass at boot");
    log::info!("  [ok] signing::self_test() passed");
}

fn test_signing_extract_sig_none_for_empty_slice() {
    assert!(
        crate::signing::extract_sig(&[]).is_none(),
        "extract_sig on empty slice must return None"
    );
    log::info!("  [ok] extract_sig(&[]) → None");
}

fn test_signing_extract_sig_none_for_non_elf() {
    // 64 bytes that do NOT start with the ELF magic — should return None gracefully.
    let garbage = [0x42u8; 64];
    assert!(
        crate::signing::extract_sig(&garbage).is_none(),
        "extract_sig on non-ELF bytes must return None"
    );
    log::info!("  [ok] extract_sig(non-elf) → None");
}

/// Verify that `extract_sig` correctly finds a `__ViCell_sig` section in a
/// handcrafted minimal RISC-V ELF64 binary.
///
/// Layout (408 bytes total):
///   [0..64]    ELF64 header (e_phnum=1, e_shnum=3, e_shstrndx=2)
///   [64..120]  PT_LOAD program header  (p_offset=120, p_filesz=8)
///   [120..128] "code" bytes            (8 zero bytes)
///   [128..192] Section header 0: NULL
///   [192..256] Section header 1: __ViCell_sig  (sh_offset=320, sh_size=64)
///   [256..320] Section header 2: .shstrtab     (sh_offset=384, sh_size=24)
///   [320..384] Signature bytes         (64 × 0xAB — sentinel value for test)
///   [384..408] String table            (\0__ViCell_sig\0.shstrtab\0)
fn test_signing_extract_sig_some_from_constructed_elf() {
    let elf = build_minimal_signed_elf([0xABu8; 64]);
    let result = crate::signing::extract_sig(&elf);
    assert!(result.is_some(), "extract_sig must find __ViCell_sig in minimal ELF");
    let extracted = result.unwrap();
    assert!(
        extracted.iter().all(|&b| b == 0xAB),
        "extracted signature bytes must match embedded sentinel (0xAB×64)"
    );
    log::info!("  [ok] extract_sig(constructed elf) → Some([0xAB; 64])");
}

fn test_signing_required_flag_off_in_dev_build() {
    // In dev builds (default features) `signing-required` must be off so that
    // unsigned cell binaries can still boot. This ensures the dev build stays
    // permissive while `signing-required` in CI is explicit and deliberate.
    assert!(
        !crate::signing::signing_required(),
        "signing_required() must be false in dev builds (feature `signing-required` not set)"
    );
    log::info!("  [ok] signing_required() → false in dev build");
}

/// Build a 408-byte minimal RISC-V ELF64 with one PT_LOAD segment and a
/// `__ViCell_sig` section carrying `sig` as its data. Used only by tests.
fn build_minimal_signed_elf(sig: [u8; 64]) -> alloc::vec::Vec<u8> {
    // String table: \0__ViCell_sig\0.shstrtab\0 (24 bytes)
    //   strtab[1]  = "__ViCell_sig\0"  → sh_name for section 1
    //   strtab[14] = ".shstrtab\0"     → sh_name for section 2 (itself)
    const STRTAB: &[u8] = b"\x00__ViCell_sig\x00.shstrtab\x00";
    let mut v = alloc::vec![0u8; 384 + STRTAB.len()]; // 384 + 24 = 408

    // ── ELF64 header (offset 0, 64 bytes) ────────────────────────────────────
    v[0..4].copy_from_slice(b"\x7fELF");
    v[4] = 2; // ELFCLASS64
    v[5] = 1; // ELFDATA2LSB
    v[6] = 1; // EV_CURRENT
    // e_type=2 (ET_EXEC), e_machine=0xF3 (EM_RISCV)
    v[16..18].copy_from_slice(&2u16.to_le_bytes());
    v[18..20].copy_from_slice(&0xF3u16.to_le_bytes());
    // e_version=1
    v[20..24].copy_from_slice(&1u32.to_le_bytes());
    // e_entry=0x1000, e_phoff=64, e_shoff=128
    v[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
    v[32..40].copy_from_slice(&64u64.to_le_bytes());
    v[40..48].copy_from_slice(&128u64.to_le_bytes());
    // e_flags=5 (RVC + double-float ABI), e_ehsize=64, e_phentsize=56, e_phnum=1
    v[48..52].copy_from_slice(&5u32.to_le_bytes());
    v[52..54].copy_from_slice(&64u16.to_le_bytes());
    v[54..56].copy_from_slice(&56u16.to_le_bytes());
    v[56..58].copy_from_slice(&1u16.to_le_bytes());
    // e_shentsize=64, e_shnum=3, e_shstrndx=2
    v[58..60].copy_from_slice(&64u16.to_le_bytes());
    v[60..62].copy_from_slice(&3u16.to_le_bytes());
    v[62..64].copy_from_slice(&2u16.to_le_bytes());

    // ── PT_LOAD program header (offset 64, 56 bytes) ─────────────────────────
    // ELF64 Phdr: p_type(4), p_flags(4), p_offset(8), p_vaddr(8), p_paddr(8),
    //             p_filesz(8), p_memsz(8), p_align(8)
    v[64..68].copy_from_slice(&1u32.to_le_bytes());       // p_type = PT_LOAD
    v[68..72].copy_from_slice(&5u32.to_le_bytes());       // p_flags = R|X
    v[72..80].copy_from_slice(&120u64.to_le_bytes());     // p_offset = 120
    v[80..88].copy_from_slice(&0x1000u64.to_le_bytes());  // p_vaddr
    v[88..96].copy_from_slice(&0x1000u64.to_le_bytes());  // p_paddr
    v[96..104].copy_from_slice(&8u64.to_le_bytes());      // p_filesz = 8
    v[104..112].copy_from_slice(&8u64.to_le_bytes());     // p_memsz = 8
    v[112..120].copy_from_slice(&0x1000u64.to_le_bytes()); // p_align

    // [120..128]: code bytes, already zero.

    // ── Section header 0: NULL (offset 128, 64 bytes) — all zero ─────────────

    // ── Section header 1: __ViCell_sig (offset 192, 64 bytes) ────────────────
    // ELF64 Shdr: sh_name(4), sh_type(4), sh_flags(8), sh_addr(8), sh_offset(8),
    //             sh_size(8), sh_link(4), sh_info(4), sh_addralign(8), sh_entsize(8)
    v[192..196].copy_from_slice(&1u32.to_le_bytes());   // sh_name = strtab[1]
    v[196..200].copy_from_slice(&1u32.to_le_bytes());   // sh_type = SHT_PROGBITS
    // sh_flags = 0 (no ALLOC — sig section must never be mapped), already zero
    // sh_offset = 320 (sig data starts there)
    v[216..224].copy_from_slice(&320u64.to_le_bytes());
    v[224..232].copy_from_slice(&64u64.to_le_bytes());  // sh_size = 64
    v[240..248].copy_from_slice(&1u64.to_le_bytes());   // sh_addralign = 1

    // ── Section header 2: .shstrtab (offset 256, 64 bytes) ───────────────────
    v[256..260].copy_from_slice(&14u32.to_le_bytes());  // sh_name = strtab[14]
    v[260..264].copy_from_slice(&3u32.to_le_bytes());   // sh_type = SHT_STRTAB
    // sh_offset = 384
    v[280..288].copy_from_slice(&384u64.to_le_bytes());
    v[288..296].copy_from_slice(&(STRTAB.len() as u64).to_le_bytes()); // sh_size
    v[304..312].copy_from_slice(&1u64.to_le_bytes());   // sh_addralign = 1

    // ── Signature bytes (offset 320, 64 bytes) ────────────────────────────────
    v[320..384].copy_from_slice(&sig);

    // ── String table (offset 384, 24 bytes) ───────────────────────────────────
    v[384..384 + STRTAB.len()].copy_from_slice(STRTAB);

    v
}
