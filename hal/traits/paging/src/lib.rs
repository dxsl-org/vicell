#![no_std]

use types::*; // use VAddr, PhysAddr, ViResult

/// Page Table Entry Flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageFlags(pub usize);

impl PageFlags {
    // Standard flags (mapping to typical HW bits)
    pub const VALID: usize = 1 << 0;
    pub const READ: usize = 1 << 1;
    pub const WRITE: usize = 1 << 2;
    pub const EXECUTE: usize = 1 << 3;
    pub const USER: usize = 1 << 4;
    pub const GLOBAL: usize = 1 << 5;
    pub const ACCESSED: usize = 1 << 6;
    pub const DIRTY: usize = 1 << 7;
    /// Device MMIO mapping — use non-cacheable Device-nGnRnE attributes (AArch64 MAIR index 0).
    /// On RISC-V and x86_64 this flag is ignored (all MMIO uses the same PTE path).
    pub const DEVICE: usize = 1 << 8;

    pub const R_W_X: usize = Self::READ | Self::WRITE | Self::EXECUTE;

    pub fn from_bits(bits: usize) -> Self {
        Self(bits)
    }

    pub fn bits(&self) -> usize {
        self.0
    }

    pub fn is_valid(&self) -> bool {
        (self.0 & Self::VALID) != 0
    }
    pub fn is_writeable(&self) -> bool {
        (self.0 & Self::WRITE) != 0
    }
    // ... add more as needed
}

/// Generic Page Table Trait
pub trait PageTableTrait {
    /// Initialize paging (e.g., allocate root table).
    /// Typically returns the Physical Address of the root table.
    fn init(&mut self) -> ViResult<PhysAddr>;

    /// Map a virtual page to a physical frame.
    /// `alloc_fn` is a closure to allocate new frames for intermediate tables.
    fn map(
        &mut self,
        virt: VAddr,
        phys: PhysAddr,
        flags: PageFlags,
        alloc_fn: &mut dyn FnMut() -> Option<PhysAddr>,
    ) -> ViResult<()>;

    /// Unmap a virtual page.
    fn unmap(&mut self, virt: VAddr) -> ViResult<()>;

    /// Translate virtual address to physical address.
    fn translate(&self, virt: VAddr) -> Option<PhysAddr>;

    /// Identity map a region (Virtual = Physical).
    fn identity_map(
        &mut self,
        start: PhysAddr,
        end: PhysAddr,
        flags: PageFlags,
        alloc_fn: &mut dyn FnMut() -> Option<PhysAddr>,
    ) -> ViResult<()> {
        let mut addr = start;
        // Assume generic page size 4KB for default identity map loop,
        // implementation can override for huge pages optimization.
        while addr < end {
            self.map(addr, addr, flags, alloc_fn)?;
            addr += 4096;
        }
        Ok(())
    }

    /// Activate this page table (load into CR3/SATP).
    /// # Safety
    /// Must ensure code/stack is mapped.
    unsafe fn activate(&self);
}
