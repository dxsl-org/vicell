//! Cell capability manifest embedded in the `__ViCell_manifest` ELF section.
//!
//! A fixed 8-byte `#[repr(C)]` record declaring which privileged capabilities a
//! Cell requests.  The kernel reads it at spawn time (see
//! `kernel/src/loader.rs::spawn_from_path`) to grant capability tokens and to
//! reject user Cells that over-declare privilege.
//!
//! Binary layout (8 bytes, little-endian):
//! ```text
//!   offset 0–3: magic   u32  = MANIFEST_MAGIC  (0x5649_4345)
//!   offset   4: version u8   = MANIFEST_VERSION (1)
//!   offset   5: flags   u8   = bitwise-OR of MANIFEST_FLAG_*
//!   offset 6–7: _pad   [u8;2]= 0x00 0x00  (reserved)
//! ```

/// Magic value identifying a valid manifest (`0x5649_4345`, "VICE" as a u32).
pub const MANIFEST_MAGIC: u32 = 0x5649_4345;

/// Current manifest layout version.  Bump on any field addition or reorder.
pub const MANIFEST_VERSION: u8 = 1;

/// Raw block-device access (BlkRead/BlkWrite/BlkFlush).  Grants `BlockIoCap`.
pub const MANIFEST_FLAG_BLOCK_IO: u8 = 1 << 0;

/// Network transmit/receive (NetTx/NetRx).  Grants `NetworkCap`.
pub const MANIFEST_FLAG_NETWORK: u8 = 1 << 1;

/// Cell spawning and hot-swap (SpawnFromPath/SpawnPinned/HotSwap).  Grants `SpawnCap`.
pub const MANIFEST_FLAG_SPAWN: u8 = 1 << 2;

/// GPIO pin control (ViGpio driver cell — PL061 on QEMU ARM virt).
/// Grants access to the GPIO MMIO range via `sys_request_mmio`.
pub const MANIFEST_FLAG_GPIO: u8 = 1 << 3;

/// UART serial access (ViUart driver cell — PL011 on QEMU ARM virt).
/// Grants access to the UART MMIO range via `sys_request_mmio`.
pub const MANIFEST_FLAG_UART: u8 = 1 << 4;

/// RISC-V H-extension (hypervisor) CSR access for VMM cells.
/// Grants `HypervisorCap` only when the CPU also reports H-ext at boot.
pub const MANIFEST_FLAG_HYPERVISOR: u8 = 1 << 5;

/// Block-I/O sector range grant: MBR partition P1 (FAT32, `api::disk`).
/// Refines `MANIFEST_FLAG_BLOCK_IO` — the cap says a cell MAY issue raw block
/// syscalls; the PART_* bits say WHICH partition's LBA range they may touch
/// (kernel `check_block_access`, deny-by-default for manifest-carrying cells).
/// P2 (cell table) and P3 (snapshot) are kernel-only and never grantable.
pub const MANIFEST_FLAG_PART_DATA: u8 = 1 << 6;

/// Block-I/O sector range grant: MBR partition P4 (littlefs, `api::disk`).
pub const MANIFEST_FLAG_PART_LFS: u8 = 1 << 7;

/// Bitmask of all defined flags for version 1.
///
/// `from_bytes` rejects manifests where `flags & !MANIFEST_FLAGS_MASK != 0` —
/// a stale v1 binary accidentally setting an undefined bit (e.g., from a future
/// SDK) is rejected and falls back to legacy path grants, preventing a
/// capability it never intended from silently activating on an older kernel.
/// (Kernels older than the PART_* bits reject manifests that set them — the
/// cell then runs on legacy path grants, which is the fail-safe direction.)
pub const MANIFEST_FLAGS_MASK: u8 =
    MANIFEST_FLAG_BLOCK_IO | MANIFEST_FLAG_NETWORK | MANIFEST_FLAG_SPAWN
    | MANIFEST_FLAG_GPIO | MANIFEST_FLAG_UART | MANIFEST_FLAG_HYPERVISOR
    | MANIFEST_FLAG_PART_DATA | MANIFEST_FLAG_PART_LFS;

/// Fixed-layout capability manifest.  ABI-stable — see Law 1.
///
/// Always 8 bytes due to `#[repr(C)]` and explicit padding.  Version the struct
/// via `MANIFEST_VERSION` before adding fields.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct CellManifest {
    /// Must equal `MANIFEST_MAGIC`; `from_bytes` rejects any other value.
    pub magic: u32,
    /// Must equal `MANIFEST_VERSION`; forward-compatibility gate.
    pub version: u8,
    /// Bitwise-OR of `MANIFEST_FLAG_*` constants.
    pub flags: u8,
    /// Reserved — must be `[0, 0]`.
    pub _pad: [u8; 2],
}

impl CellManifest {
    /// Construct a manifest from capability bits.
    ///
    /// Evaluates at compile time; safe to use as a `static` initializer.
    pub const fn new(
        block_io: bool, network: bool, spawn: bool,
        gpio: bool, uart: bool, hypervisor: bool,
    ) -> Self {
        Self::with_parts(block_io, network, spawn, gpio, uart, hypervisor, false, false)
    }

    /// Construct a manifest including block-I/O partition range grants.
    ///
    /// `part_data`/`part_lfs` only have effect when `block_io` is also set —
    /// they scope WHICH partition the raw block syscalls may address.
    #[allow(clippy::too_many_arguments)] // mirrors the flat flag layout; macro is the public face
    pub const fn with_parts(
        block_io: bool, network: bool, spawn: bool,
        gpio: bool, uart: bool, hypervisor: bool,
        part_data: bool, part_lfs: bool,
    ) -> Self {
        Self {
            magic:   MANIFEST_MAGIC,
            version: MANIFEST_VERSION,
            flags:   (block_io   as u8 * MANIFEST_FLAG_BLOCK_IO)
                   | (network    as u8 * MANIFEST_FLAG_NETWORK)
                   | (spawn      as u8 * MANIFEST_FLAG_SPAWN)
                   | (gpio       as u8 * MANIFEST_FLAG_GPIO)
                   | (uart       as u8 * MANIFEST_FLAG_UART)
                   | (hypervisor as u8 * MANIFEST_FLAG_HYPERVISOR)
                   | (part_data  as u8 * MANIFEST_FLAG_PART_DATA)
                   | (part_lfs   as u8 * MANIFEST_FLAG_PART_LFS),
            _pad:    [0; 2],
        }
    }

    /// Parse a manifest from raw ELF section bytes.
    ///
    /// Field-by-field — never casts the slice to `&Self` (alignment hazard in
    /// `no_std`).
    ///
    /// # Returns
    /// `None` if the slice is shorter than 8 bytes, the magic mismatches, or the
    /// version is unsupported.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }
        let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if magic != MANIFEST_MAGIC {
            return None;
        }
        if bytes[4] != MANIFEST_VERSION {
            return None;
        }
        let flags = bytes[5];
        // Reject reserved flag bits — a stale binary setting a future flag is
        // treated as malformed rather than silently gaining an unintended cap.
        if flags & !MANIFEST_FLAGS_MASK != 0 {
            return None;
        }
        Some(Self {
            magic,
            version: bytes[4],
            flags,
            _pad:    [bytes[6], bytes[7]],
        })
    }

    /// Returns `true` if the cell declared raw block-device access.
    pub fn has_block_io(&self) -> bool { self.flags & MANIFEST_FLAG_BLOCK_IO != 0 }

    /// Returns `true` if the cell declared network transmit/receive.
    pub fn has_network(&self) -> bool { self.flags & MANIFEST_FLAG_NETWORK != 0 }

    /// Returns `true` if the cell declared cell-spawning and hot-swap.
    pub fn has_spawn(&self) -> bool { self.flags & MANIFEST_FLAG_SPAWN != 0 }

    /// Returns `true` if the cell declared GPIO pin-control access.
    pub fn has_gpio(&self) -> bool { self.flags & MANIFEST_FLAG_GPIO != 0 }

    /// Returns `true` if the cell declared UART serial access.
    pub fn has_uart(&self) -> bool { self.flags & MANIFEST_FLAG_UART != 0 }

    /// Returns `true` if the cell declared H-extension hypervisor CSR access.
    pub fn has_hypervisor(&self) -> bool { self.flags & MANIFEST_FLAG_HYPERVISOR != 0 }

    /// Returns `true` if the cell's block I/O is granted the P1 (FAT32) range.
    pub fn has_part_data(&self) -> bool { self.flags & MANIFEST_FLAG_PART_DATA != 0 }

    /// Returns `true` if the cell's block I/O is granted the P4 (littlefs) range.
    pub fn has_part_lfs(&self) -> bool { self.flags & MANIFEST_FLAG_PART_LFS != 0 }

    /// Returns `true` if any privileged capability bit is set.
    ///
    /// Used by `spawn_from_path` to reject over-declaring user Cells (non-`/bin/`
    /// paths).
    pub fn declares_any_privilege(&self) -> bool { self.flags != 0 }
}

/// Embed a capability manifest into the current Cell's ELF binary.
///
/// Places a fixed 8-byte `CellManifest` into the `__ViCell_manifest` ELF section.
/// The cell linker script must `KEEP` that section or `--gc-sections` will
/// silently drop it in release/LTO builds.
///
/// # Usage
/// ```ignore
/// // At module scope, after `use` declarations:
/// api::declare_manifest!(block_io = true, network = false, spawn = false, gpio = false, uart = false);
/// ```
#[macro_export]
macro_rules! declare_manifest {
    // Full form — includes block-I/O partition range grants (P1 data / P4 lfs).
    (block_io = $bio:literal, network = $net:literal, spawn = $spawn:literal, gpio = $gpio:literal, uart = $uart:literal, hypervisor = $hv:literal, part_data = $pd:literal, part_lfs = $pl:literal) => {
        #[used]
        #[link_section = "__ViCell_manifest"]
        pub static VICELL_MANIFEST: $crate::manifest::CellManifest =
            $crate::manifest::CellManifest::with_parts($bio, $net, $spawn, $gpio, $uart, $hv, $pd, $pl);
    };
    // Convenience form: block_io with partition scopes, no gpio/uart/hypervisor.
    (block_io = $bio:literal, network = $net:literal, spawn = $spawn:literal, part_data = $pd:literal, part_lfs = $pl:literal) => {
        $crate::declare_manifest!(block_io = $bio, network = $net, spawn = $spawn, gpio = false, uart = false, hypervisor = false, part_data = $pd, part_lfs = $pl);
    };
    // 6-param form — includes hypervisor flag.
    (block_io = $bio:literal, network = $net:literal, spawn = $spawn:literal, gpio = $gpio:literal, uart = $uart:literal, hypervisor = $hv:literal) => {
        #[used]
        #[link_section = "__ViCell_manifest"]
        pub static VICELL_MANIFEST: $crate::manifest::CellManifest =
            $crate::manifest::CellManifest::new($bio, $net, $spawn, $gpio, $uart, $hv);
    };
    // 5-param form (no hypervisor) — hypervisor defaults to false.
    (block_io = $bio:literal, network = $net:literal, spawn = $spawn:literal, gpio = $gpio:literal, uart = $uart:literal) => {
        $crate::declare_manifest!(block_io = $bio, network = $net, spawn = $spawn, gpio = $gpio, uart = $uart, hypervisor = false);
    };
    // 3-param back-compat form (no gpio/uart/hypervisor) — all default to false.
    (block_io = $bio:literal, network = $net:literal, spawn = $spawn:literal) => {
        $crate::declare_manifest!(block_io = $bio, network = $net, spawn = $spawn, gpio = false, uart = false, hypervisor = false);
    };
}
