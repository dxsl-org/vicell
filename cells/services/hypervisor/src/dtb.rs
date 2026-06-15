//! Minimal DTB builder for the Alpine Linux guest.
//!
//! Emits the 7 mandatory nodes using `vm-fdt`. Missing any node causes a silent
//! guest panic during Linux GIC/timer/console init (R4 risk #1, HIGHEST severity).
//!
//! Node list (from phase-05 Key Insights):
//! 1. /memory@40000000  — guest RAM
//! 2. /cpus/cpu@0       — cortex-a72, enable-method = "psci"
//! 3. /psci             — method = "hvc", compatible = "arm,psci-1.0"
//! 4. /intc             — GICv2 (arm,cortex-a15-gic), GICD@0x08000000 GICC@0x08010000
//! 5. /timer            — armv8-timer PPIs 13/14/11/10, level-low
//! 6. /chosen           — bootargs, initrd-start/end, stdout-path
//! 7. /pl011@9000000    — arm,pl011 console

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use vm_fdt::{FdtWriter, FdtWriterResult};

/// Boot arguments passed to the Linux kernel command line.
pub const BOOTARGS: &str =
    "console=hvc0 console=ttyAMA0 earlycon=pl011,0x9000000 rdinit=/bin/sh panic=1 loglevel=8";

/// Build a minimal DTB for a single-CPU Alpine guest.
///
/// Parameters:
/// - `ram_base` / `ram_size`: guest physical RAM region (usually 0x40000000, configurable MB)
/// - `initrd_start` / `initrd_end`: guest physical addresses of the initramfs blob
///
/// Returns the raw DTB bytes.
pub fn build_dtb(
    ram_base: u64,
    ram_size: u64,
    initrd_start: u64,
    initrd_end: u64,
) -> FdtWriterResult<Vec<u8>> {
    let mut fdt = FdtWriter::new()?;

    let root = fdt.begin_node("")?;
    fdt.property_u32("#address-cells", 2)?;
    fdt.property_u32("#size-cells", 2)?;
    fdt.property_string("compatible", "linux,dummy-virt")?;

    // ── 1. /memory ───────────────────────────────────────────────────────────
    let mem = fdt.begin_node(&alloc::format!("memory@{:x}", ram_base))?;
    fdt.property_string("device_type", "memory")?;
    // reg = <base_hi base_lo size_hi size_lo>
    fdt.property_array_u32("reg", &[
        (ram_base >> 32) as u32, (ram_base & 0xFFFF_FFFF) as u32,
        (ram_size >> 32) as u32, (ram_size & 0xFFFF_FFFF) as u32,
    ])?;
    fdt.end_node(mem)?;

    // ── 2. /cpus ─────────────────────────────────────────────────────────────
    let cpus = fdt.begin_node("cpus")?;
    fdt.property_u32("#address-cells", 1)?;
    fdt.property_u32("#size-cells", 0)?;

    let cpu0 = fdt.begin_node("cpu@0")?;
    fdt.property_string("device_type", "cpu")?;
    fdt.property_string("compatible", "arm,cortex-a72")?;
    fdt.property_string("enable-method", "psci")?;
    fdt.property_u32("reg", 0)?;
    fdt.end_node(cpu0)?;
    fdt.end_node(cpus)?;

    // ── 3. /psci ─────────────────────────────────────────────────────────────
    let psci = fdt.begin_node("psci")?;
    // arm,psci-1.0 compatible — must use "hvc" method (not "smc").
    fdt.property_string_list("compatible", vec![
        alloc::string::String::from("arm,psci-1.0"),
        alloc::string::String::from("arm,psci-0.2"),
        alloc::string::String::from("arm,psci"),
    ])?;
    fdt.property_string("method", "hvc")?;
    // PSCI 1.0 function IDs (for SMCCC 32-bit / hvc call convention).
    fdt.property_u32("cpu_suspend", 0x8400_0001)?;
    fdt.property_u32("cpu_off",     0x8400_0002)?;
    fdt.property_u32("cpu_on",      0x8400_0003)?;
    fdt.property_u32("migrate",     0x8400_0005)?;
    fdt.end_node(psci)?;

    // ── 4. /intc — GICv2 ─────────────────────────────────────────────────────
    // reg: GICD base (64KiB) + GICC base (64KiB).
    // arm,cortex-a15-gic is the compatible string for GICv2 used by QEMU virt.
    let intc = fdt.begin_node("intc@8000000")?;
    fdt.property_string_list("compatible", vec![
        alloc::string::String::from("arm,cortex-a15-gic"),
        alloc::string::String::from("arm,gic-400"),
    ])?;
    fdt.property_null("interrupt-controller")?;
    fdt.property_u32("#interrupt-cells", 3)?;
    fdt.property_u32("#address-cells", 0)?;
    fdt.property_array_u32("reg", &[
        0x0, 0x0800_0000, 0x0, 0x0001_0000,  // GICD
        0x0, 0x0801_0000, 0x0, 0x0001_0000,  // GICC
    ])?;
    let intc_phandle = fdt.property_phandle(1)?;
    let _ = intc_phandle; // phandle=1 for interrupt-parent references
    fdt.end_node(intc)?;

    // ── 5. /timer — armv8-timer ──────────────────────────────────────────────
    // PPIs: secure EL1 (13), non-secure EL1 (14), virtual (11), hypervisor (10).
    // All level-low (GIC_PPI | IRQ_TYPE_LEVEL_LOW = <1 N 8>).
    let timer = fdt.begin_node("timer")?;
    fdt.property_string("compatible", "arm,armv8-timer")?;
    fdt.property_u32("interrupt-parent", 1)?; // phandle of /intc
    // Each PPI = <GIC_PPI irq flags>; GIC_PPI=1, flags=0x8=IRQ_TYPE_LEVEL_LOW.
    fdt.property_array_u32("interrupts", &[
        1, 13, 0x8,  // secure EL1 PPI 13
        1, 14, 0x8,  // non-secure EL1 PPI 14
        1, 11, 0x8,  // virtual PPI 11
        1, 10, 0x8,  // hypervisor PPI 10
    ])?;
    fdt.property_null("always-on")?;
    fdt.end_node(timer)?;

    // ── 6. /chosen ───────────────────────────────────────────────────────────
    let chosen = fdt.begin_node("chosen")?;
    fdt.property_string("bootargs", BOOTARGS)?;
    fdt.property_string("stdout-path", "pl011@9000000")?;
    fdt.property_u64("linux,initrd-start", initrd_start)?;
    fdt.property_u64("linux,initrd-end",   initrd_end)?;
    fdt.end_node(chosen)?;

    // ── 7. /pl011@9000000 ────────────────────────────────────────────────────
    let uart = fdt.begin_node("pl011@9000000")?;
    fdt.property_string_list("compatible", vec![
        alloc::string::String::from("arm,pl011"),
        alloc::string::String::from("arm,primecell"),
    ])?;
    fdt.property_u32("interrupt-parent", 1)?;
    // UART0 SPI 1 level-high = <0 1 4> (GIC_SPI=0, irq=1, IRQ_TYPE_LEVEL_HIGH=4).
    fdt.property_array_u32("interrupts", &[0, 1, 4])?;
    fdt.property_array_u32("reg", &[0x0, 0x0900_0000, 0x0, 0x1000])?;
    fdt.property_u32("clock-frequency", 0x16E360)?; // 1.5 MHz (QEMU default)
    fdt.end_node(uart)?;

    // ── 8. /virtio_mmio@a000000 — console (slot 0, SPI 16) ──────────────────
    let vio = fdt.begin_node("virtio_mmio@a000000")?;
    fdt.property_string("compatible", "virtio,mmio")?;
    fdt.property_u32("interrupt-parent", 1)?;
    // SPI 16, IRQ_TYPE_EDGE_RISING=1: <GIC_SPI=0, irq=16, flags=1>
    fdt.property_array_u32("interrupts", &[0, 16, 1])?;
    fdt.property_array_u32("reg", &[0x0, 0x0a00_0000, 0x0, 0x200])?;
    fdt.end_node(vio)?;

    fdt.end_node(root)?;
    fdt.finish()
}
