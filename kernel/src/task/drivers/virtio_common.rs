//! Shared VirtIO MMIO slot enumeration.
//!
//! Both `virtio_blk`, `virtio_input`, and `virtio_net` use `virtio_slots()` to
//! iterate all VirtIO MMIO slots for the current platform.
//!
//! AArch64: scans all 32 slots at 0x0a000000, stride 0x200 (QEMU virt layout).
//! QEMU assigns devices to slots in an implementation-defined order so we must
//! probe all 32.  The identity map in paging.rs covers the full 0x0a004000 range.
//!
//! Other arches: reads DTB-confirmed slots from `platform::PLATFORM`.

extern crate alloc;
use alloc::vec::Vec;

/// A VirtIO MMIO slot with base address and IRQ.
pub struct VirtioSlot {
    pub base: usize,
    pub irq:  u32,
}

/// Iterator over all VirtIO MMIO slots for the current platform.
pub fn virtio_slots() -> impl Iterator<Item = VirtioSlot> {
    #[cfg(target_arch = "aarch64")]
    {
        // QEMU ARM virt: 32 VirtIO MMIO slots at 0x0a000000, 512 bytes each, SPI 16+i.
        // All 32 slots are identity-mapped by init_kernel_paging (0x0a000000..0x0a004000).
        const BASE: usize   = 0x0a00_0000;
        const STRIDE: usize = 0x200;
        let slots: Vec<VirtioSlot> = (0..32_usize)
            .map(|i| VirtioSlot { base: BASE + i * STRIDE, irq: 16 + i as u32 })
            .collect();
        return slots.into_iter();
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let slots: Vec<VirtioSlot> = crate::platform::with(|p| {
            p.virtio_mmio
                .iter()
                .filter_map(|e| e.as_ref().map(|e| VirtioSlot { base: e.base, irq: e.irq }))
                .collect()
        });
        slots.into_iter()
    }
}
