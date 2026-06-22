// SPDX-License-Identifier: MPL-2.0
//! Cellos Kernel - Entry point

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::panic::PanicInfo;

// Core kernel modules
pub mod acpi;
pub mod audit;
pub mod boot;
pub mod cell;
pub mod ed25519; // Ed25519 verify (no_std) for signed operator policy (P5 spike)
pub mod hypervisor; // EL2 VMM kernel support (Phase 03+)
pub mod resource_registry;
pub mod fast_ipc; // Kernel-owned fast-IPC dispatch table (canonical instance)
pub mod fs; // Filesystem
pub mod loader;
pub mod measurement_log; // Per-Cell integrity measurement (IMA-style, TPM-free)
pub mod memory;
pub mod policy; // Signed operator policy (P5b) — headless consent
pub mod sha256; // Self-contained SHA-256 for measurement
pub mod snapshot;
pub mod task; // Renamed from 'process'
              // pub mod arch; // Moved to HAL
pub extern crate hal; // HAL (Architecture specific)
use boot::BootInfo;
use hal::Arch;

// Internal utilities
mod cpu_features;
mod sync;
pub mod platform;

/// Signal QEMU to exit with a success (0) or failure (1) code.
///
/// Only available under the `test-hooks` feature — never call this in
/// production builds. The kernel integration-test harness uses this
/// instead of parsing serial output for "PASS"/"FAIL" banners.
///
/// Device addresses: RISC-V = SiFive test 0x100000, ARM64 = semihosting,
/// x86_64 = isa-debug-exit (iobase 0xF4).
#[cfg(feature = "test-hooks")]
pub fn qemu_exit(success: bool) -> ! {
    use qemu_exit::QEMUExit;
    #[cfg(target_arch = "riscv64")]
    { qemu_exit::RISCV64::new(0x100000).exit(if success { 0 } else { 1 }); }
    #[cfg(target_arch = "aarch64")]
    { qemu_exit::AArch64Semihosting::default().exit(if success { 0 } else { 1 }); }
    #[cfg(target_arch = "x86_64")]
    { qemu_exit::X86::new(0xF4, 0).exit(if success { 0 } else { 1 }); }
    // Fallback for other arches: spin forever so the test times out clearly.
    #[allow(clippy::empty_loop)]
    loop {}
}

// Re-export types for convenience
pub use types::*;

// Embed Init Binary (stripped by build.rs, served from EMBEDDED_OUT_DIR).
// RV32 Nano (Phase 31) has no init ELF; x86_64 is now included (Phase 04).
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64"))]
static INIT_ELF: &[u8] = include_bytes!(concat!(env!("EMBEDDED_OUT_DIR"), "/init"));

/// Kernel entry point called from HAL boot code
#[no_mangle]
pub extern "C" fn kmain(hartid: usize, dtb: usize) -> ! {
    let _hartid = hartid;
    cpu_features::detect(dtb);
    // Parse DTB for MMIO bases before any driver or paging init.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    crate::platform::init(dtb);
    // Set runtime PLIC base before hal::ARCH.init() calls plic::init() internally.
    #[cfg(target_arch = "riscv64")]
    crate::platform::with(|p| hal::common::plic::set_plic_base(p.plic_base));
    // 0. Initialize UART immediately for early logging
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    task::drivers::uart::init();
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    crate::hal::uart_pl011::init();
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    crate::hal::uart_16550::init();

    // Set HHDM base for LAPIC/IOAPIC MMIO access AND for phys_to_virt.
    // Limine maps RAM at HHDM_BASE+phys (no identity mapping of physical RAM).
    // This must be called before FrameAllocator::new_from_map.
    #[cfg(target_arch = "x86_64")]
    {
        let hhdm = crate::boot::limine::get_hhdm_offset().unwrap_or(0);
        crate::hal::apic::set_hhdm_base(hhdm);
        crate::memory::frame::set_phys_offset(hhdm as usize);
        // Propagate the HHDM offset to the HAL PML4 walker so walk_create /
        // walk_read can dereference physical PTE addresses via HHDM virtual ptrs.
        crate::hal::paging::set_hhdm_offset(hhdm as usize);
        // Limine maps only usable e820 RAM in the HHDM. The BIOS ROM area
        // [0x9F000–0x100000) is reserved in e820 and absent from Limine's HHDM
        // page table. ACPI RSDP is typically there (~0xf52e0 on q35).
        // Map it now so phys_to_virt(rsdp) doesn't triple-fault before IDT is up.
        // SAFETY: called after set_hhdm_offset; PML4 walker uses HHDM-mapped RAM.
        unsafe { crate::hal::paging::map_bios_area(); }
        // Initialise KASLR seed from HHDM entropy + RDTSC.
        crate::memory::kaslr::init_kaslr(hhdm);
    }

    // Parse ACPI tables BEFORE paging init so we have the real MMIO addresses
    // for LAPIC, IOAPIC, HPET, and PCIe ECAM.  Must run after HHDM offset is set
    // (phys_to_virt requires it) but before init_kernel_paging_x86 maps MMIO.
    //
    // On failure or absent RSDP the parser returns QEMU q35 defaults so the
    // system boots unchanged on emulated hardware.
    #[cfg(target_arch = "x86_64")]
    let acpi_info = {
        use crate::memory::frame::phys_to_virt;
        let early_puts = |s: &str| {
            for c in s.bytes() { crate::hal::uart_16550::putchar(c); }
        };
        let rsdp = crate::boot::limine::get_rsdp_ptr().unwrap_or(0);
        // Limine 8.x maps only usable e820 RAM in its HHDM.  The BIOS ROM area
        // [0x0000–0x100000) and ACPI-reserved regions near top-of-RAM are absent.
        // Accessing those regions via HHDM before the IDT is live triple-faults.
        // On QEMU q35 the RSDP is always in the BIOS area (< 1 MiB), and the XSDT
        // lives in the ACPI-reserved window — neither is reachable this early.
        // Use hardcoded q35 defaults here; TODO: re-parse after init_kernel_paging
        // creates a full physical window that covers all e820 regions.
        if rsdp != 0 && rsdp >= 0x10_0000 {
            let info = crate::acpi::parse(rsdp, |p| phys_to_virt(p));
            early_puts("[INFO] ACPI tables parsed\n");
            info
        } else {
            if rsdp == 0 {
                early_puts("[INFO] ACPI RSDP absent — using q35 defaults\n");
            } else {
                early_puts("[INFO] ACPI RSDP in BIOS area — using q35 defaults\n");
            }
            crate::acpi::AcpiInfo::default()
        }
    };

    // 1. Initialize HAL (Architecture specific) - Early Trap Setup
    // x86_64: LAPIC is deferred until after paging sets up the MMIO mapping
    // (LAPIC phys 0xFEE00000 isn't in Limine's HHDM for MMIO regions).
    #[cfg(target_arch = "x86_64")]
    {
        crate::hal::gdt::init();
        crate::hal::idt::init();
        crate::hal::syscall::init();
        // apic::init_lapic() deferred — needs MMIO mapped via custom PML4
    }
    #[cfg(not(target_arch = "x86_64"))]
    hal::ARCH.init();
    // Initialize Goldfish RTC for wall-clock time on QEMU ARM64 virt.
    // Must run after ARCH.init() (which sets up MMIO identity map via paging::init).
    #[cfg(target_arch = "aarch64")]
    hal::rtc::init_default();

    // Define puts helper — arch-specific character output.
    let puts = |s: &str| {
        for c in s.bytes() {
            #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
            { let _ = crate::hal::sbi::console_putchar(c); }
            #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
            { crate::hal::uart_pl011::putchar(c); }
            #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
            { crate::hal::uart_16550::putchar(c); }
        }
    };

    // Restore log_info helper
    let log_info = |s: &str| {
        puts("[INFO] ");
        puts(s);
        puts("\n");
    };

    // Stable banner — CI greps for this exact string.
    puts("[Cellos] kernel boot v");
    puts(env!("CARGO_PKG_VERSION"));
    puts("\n");
    puts("Kernel started (Hart: 0, DTB: ...)\n");
    #[cfg(target_arch = "riscv64")]
    if cpu_features::has_h_ext() {
        puts("[cpu] H-extension: detected\n");
    } else {
        puts("[cpu] H-extension: not present\n");
    }

    // Parse bootloader information
    let boot_info_result = boot::parse_bootloader_info();

    // Check if Limine failed, if so, use fallback (SimpleBootInfo)
    let boot_info: &dyn BootInfo = match &boot_info_result {
        Ok(info) => info,
        Err(_) => {
            log_info("Limine not found, using QEMU/OpenSBI fallback");
            // Use fallback static instance (defined in boot.rs or created here)
            // For now, let's just use the fallback function we will create
            &boot::FALLBACK_BOOT_INFO
        }
    };
    // Log physical base — non-default value confirms KASLR is active.
    {
        puts("[boot] kernel_phys_base=0x");
        let mut base = boot_info.kernel_base();
        let digits = b"0123456789abcdef";
        let mut hex_buf = [b'0'; 16];
        for i in (0..16).rev() {
            hex_buf[i] = digits[base & 0xf];
            base >>= 4;
        }
        if let Ok(s) = core::str::from_utf8(&hex_buf) {
            puts(s);
        }
        puts("\n");
    }

    // Initialize kernel subsystems

    // 1. Memory Management
    // Get memory map from Boot Info (Converted to Cellos format)
    let mmap_entries = boot_info.memory_map();

    // Initialize frame allocator with the largest usable region
    let frame_allocator = memory::frame::FrameAllocator::new_from_map(mmap_entries);

    // 2. Frame Allocator (Physical Memory)
    // The local `frame_allocator` is moved into the global static.
    // A mutable reference to the global static will be used for paging setup.
    unsafe {
        core::ptr::write(
            &mut *memory::frame::FRAME_ALLOCATOR.lock(),
            Some(frame_allocator),
        );
    }
    log_info("Frame allocator initialized");

    // 3. Paging (Virtual Memory) Setup
    // x86_64 bring-up: Limine's PML4 already maps RAM via HHDM and the kernel
    // at 0xFFFFFFFF80000000. We skip building + activating our own page tables
    // until the full x86_64 port (Phase 09). init_kernel_paging uses physical
    // addresses as virtual pointers, which would fault under Limine's paging.
    #[cfg(not(any(
        target_arch = "riscv32",
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "arm",
    )))]
    {
        log_info("Initializing paging...");
        let mut locked_frame_allocator = memory::frame::FRAME_ALLOCATOR.lock();
        let root_table_phys = memory::paging::init_kernel_paging(
            locked_frame_allocator
                .as_mut()
                .expect("Frame allocator not initialized"),
            mmap_entries,
        )
        .expect("Failed to initialize paging");
        drop(locked_frame_allocator);
        log_info("Paging initialized");
        log_info("Activating paging...");
        unsafe { memory::paging::activate_paging(root_table_phys); }
        log_info("Paging activated");
    }
    #[cfg(target_arch = "x86_64")]
    {
        // Set runtime ECAM base from ACPI before PCIe scan.
        crate::task::drivers::pcie_ecam::set_ecam_base_x86(acpi_info.ecam_base as usize);

        log_info("Initializing x86_64 paging (kernel PML4)...");
        let root_table_phys = {
            let mut locked_frame_allocator = memory::frame::FRAME_ALLOCATOR.lock();
            memory::paging::init_kernel_paging_x86(
                locked_frame_allocator
                    .as_mut()
                    .expect("Frame allocator not initialized"),
                acpi_info.ioapic_base,
                acpi_info.hpet_base,
                acpi_info.lapic_base,
                acpi_info.ecam_base,
            )
            .expect("Failed to initialize x86_64 kernel PML4")
        };
        log_info("x86_64 paging initialized");
        log_info("Activating x86_64 paging (mov cr3)...");
        // SAFETY: init_kernel_paging_x86 copied higher-half entries from Limine's PML4
        // (preserving kernel text/data/HHDM) and identity-mapped MMIO, so the kernel
        // continues executing after this CR3 switch without a triple-fault.
        unsafe { memory::paging::activate_paging(root_table_phys); }
        // Immediate port-I/O probe after activate_paging — if 'Q' appears on serial,
        // the CR3 switch succeeded and execution reached kmain.  Uses direct out instruction
        // (no Rust function call) so it cannot be affected by any post-switch state issue.
        // SAFETY: port I/O to COM1 (0x3F8) is always valid from ring-0.
        unsafe {
            core::arch::asm!(
                "99: in al, dx",
                "test al, 0x20",
                "jz 99b",
                "mov dx, {thr}",
                "mov al, 0x51",   // 'Q'
                "out dx, al",
                thr = const 0x3F8u16,
                in("dx") 0x3FDu16,
                out("al") _,
                options(nomem, nostack)
            );
        }
        // Propagate ACPI-parsed MMIO bases to the APIC driver before init_timers.
        // Defaults are QEMU q35 values; on real hardware these may differ.
        crate::hal::apic::set_lapic_phys(acpi_info.lapic_base);
        crate::hal::apic::set_ioapic_phys(acpi_info.ioapic_base);
        crate::hal::apic::set_irq_overrides(&acpi_info.irq_overrides, acpi_info.ioapic_gsi_base);
        // Propagate HPET base to HAL timer init (runtime from ACPI, fallback to
        // QEMU q35 default 0xFED0_0000 when ACPI is absent).
        crate::hal::set_hpet_base(acpi_info.hpet_base as usize);
        // HPET + calibrated LAPIC periodic timer: now safe because HPET, IOAPIC,
        // and LAPIC are identity-mapped in our new PML4 at the ACPI-parsed bases.
        crate::hal::init_timers();
        log_info("x86_64 timers initialized (HPET + LAPIC)");
    }
    // Bare physical: RV32 Nano (SATP=0), x86_32 (CR0.PG=0), AArch32 (MMU off).
    #[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
    {
        memory::paging::init_bare();
        log_info("Paging: bare physical");
    }

    // 4. Heap Allocator (Global) - MUST be after paging but before any allocations
    // 32 MiB = 8192 frames. Sized to hold:
    //   - embedded RAM disk copy (~4 MiB), VirtIO GPU framebuffer (~4 MiB), cell ELFs + kernel structures
    const HEAP_FRAMES: usize = 4_096;
    let heap_start = {
        let mut allocator_guard = memory::frame::FRAME_ALLOCATOR.lock();
        let allocator = allocator_guard.as_mut().expect("Frame allocator not initialized");
        let start = allocator.allocate_frame().expect("OOM: Heap start");
        for _ in 1..HEAP_FRAMES {
            allocator.allocate_frame().expect("OOM: Heap continuation");
        }
        start
    };
    let heap_size = HEAP_FRAMES * 4096;
    // On x86_64, phys_to_virt adds HHDM offset (Limine maps RAM at HHDM+phys).
    // On RISC-V, phys_to_virt returns phys unchanged (identity-mapped before paging).
    let heap_virt = memory::frame::phys_to_virt(heap_start);
    unsafe { memory::heap::init_heap(heap_virt, heap_size); }
    log_info("Heap initialized");

    memory::rt_heap::init();
    log_info("RT heap initialized");

    // 5. Hardware Abstraction Layer (HAL) Initialization
    // GDT/IDT/SYSCALL already done at step 1. Initialize PLIC for RISC-V external IRQs.
    #[cfg(target_arch = "riscv64")]
    crate::hal::common::plic::init();
    log_info("HAL initialized (PLIC enabled)");

    // 6. Logger & Drivers & FS
    task::drivers::uart::init(); // registers log backend on all arches
    #[cfg(target_arch = "riscv64")]
    task::drivers::uart::init_input();
    // RV32 Nano / x86_64 bring-up: skip VirtIO MMIO probing (PCIe transport not yet ported).
    // x86_64 gets VirtIO via the PCI path in virtio_pci::init() below.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    task::drivers::init();
    // x86_64: load embedded kernel_fs.img into RAM so EarlyLoader can serve ELFs from it.
    // VirtIO devices on q35 are on PCIe — probed via virtio_pci::init() after ECAM scan.
    #[cfg(target_arch = "x86_64")]
    {
        task::drivers::ramdisk::init_driver();
        // Wire COM1 RX IRQ → IDT vector 0x24 → shell stdin.
        // Must run after init_timers() (which initialises the LAPIC + IOAPIC).
        crate::hal::uart_16550::init_input_irq();
        // Initialise the RX buffer that vi_handle_uart_irq() writes into.
        task::drivers::uart::init_input();
        log_info("x86_64: ramdisk + UART RX IRQ initialised");
    }

    // PCIe ECAM scan + NVMe + e1000 + VirtIO PCI init.
    // ARM64 virt uses VirtIO MMIO (not PCIe); accessing 0x3F000000 on QEMU
    // virt 7+ triggers a Synchronous External Abort — skip on aarch64.
    // x86_64 q35: ECAM base 0xB000_0000 is identity-mapped by init_kernel_paging_x86;
    // virtio_pci::init() probes vendor 0x1AF4 PCIe devices for BLK/NET.
    #[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
    {
        task::drivers::pcie_ecam::init();
        // Phase 1: probe IOMMU hardware on both PCIe arches, allocate page tables.
        // Stays passthrough until activate_isolation() — drivers register DMA ranges first.
        task::drivers::iommu::init();
        // RISC-V virt: NVMe + e1000 PCIe endpoints (allocate DMA → map_dma auto-called).
        #[cfg(target_arch = "riscv64")]
        {
            task::drivers::blk_nvme::init_driver();
            task::drivers::nic_e1000::init_driver();
        }
        // VirtIO PCI: probe vendor 0x1AF4 on PCIe bus 0.
        // On x86_64 q35, VirtIO BLK/NET are PCIe devices; on RISC-V virt,
        // VirtIO is MMIO — virtio_pci::init() is a no-op there.
        task::drivers::virtio_pci::init();
        // Phase 3: switch IOMMU from passthrough to enforcement.
        // All PCIe DMA buffers registered via map_dma(); IOVA not in those ranges → fault.
        task::drivers::iommu::activate_isolation();
    }

    // Attempt warm boot from snapshot before any cell initialization.
    // RV32 Nano / x86_64 skip: no VirtIO block in bring-up.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    if snapshot::try_restore() {
        // try_restore() called yield_cpu() and should not return in a successful
        // warm boot.  If we reach here, fall through to cold boot as a safety net.
        log::warn!("[boot] snapshot restore returned unexpectedly → cold boot");
    }

    // Cross-check the on-disk MBR against the compiled-in partition layout
    // (warn-only — surfaces image/kernel drift at boot instead of as silent
    // corruption later).
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    crate::loader::disk_layout::verify_mbr();

    // Probe the cell bootstrap table so SpawnFromPath works during init.
    // RV32 Nano / x86_64 bring-up: no disk.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    match crate::loader::early::EarlyLoader::probe() {
        Ok(()) => puts("[loader] cell bootstrap table loaded\n"),
        Err(_) => puts("[loader] WARN: cell table not found — disk image may lack bootstrap section\n"),
    }

    // RV32 Nano: no FAT32 FS in bring-up.
    // x86_64 uses the ramdisk-backed embedded FS to serve cell ELFs via VIFS1.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64"))]
    fs::init();

    // Load + verify the signed operator policy (P5b) NOW: after VIFS1 is mounted,
    // before any cap-bearing cell spawns. Absent → dev-permissive (this G1 build);
    // invalid → fail-closed. Phase 04 folds policy::lookup into the spawn grant.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64"))]
    policy::load_from_vifs1();

    // Phase 20: hot-migration state-transfer self-test.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    crate::cell::state_stash::self_test();

    log_info("Kernel subsystems initialized successfully.");

    // 7. Initialize Scheduler
    log_info("Initializing scheduler...");
    task::init();
    log_info("Scheduler initialized");

    // 7b. Bring secondary harts online (riscv64 only; no-op on other arches).
    // Must run AFTER task::init() so the heap and scheduler are live before
    // any secondary hart starts running kernel code.
    #[cfg(target_arch = "riscv64")]
    task::smp::start_secondaries();

    // 8. Spawn Embedded Init
    // RV32 Nano bring-up: no init binary — boot to idle loop.
    // x86_64 now included (Phase 04): embedded init ELF at embedded-x86_64/init.
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64"))]
    {
        log_info("Spawning Embedded Init...");

        // Enable SUM (Supervisor User Memory access) bit in sstatus (RISC-V only).
        // ARM64 EL1 can always access EL0 pages — no equivalent bit needed.
        #[cfg(target_arch = "riscv64")]
        unsafe {
            core::arch::asm!("csrs sstatus, {0}", in(reg) 0x40000);
        }

        // Power-on self-test of the Ed25519 verify primitive (RFC 8032 TEST 1 +
        // tamper-negative) before it is trusted to authenticate the signed
        // operator policy (P5). Cheap (~one verify); a FAIL means the crypto path
        // is broken and signed policy must not be trusted.
        if crate::ed25519::self_test() {
            log_info("ed25519 verify self-test PASS (RFC 8032 + tamper)");
        } else {
            log_info("ed25519 verify self-test FAIL — signed policy unsafe");
        }
        // Power-on self-test of the signed-policy path: verify + parse a known
        // dev-signed blob, and confirm a tampered blob is rejected.
        if crate::policy::self_test() {
            log_info("policy verify+parse self-test PASS (signed blob + tamper)");
        } else {
            log_info("policy verify+parse self-test FAIL");
        }

        // Copy to Vec to ensure alignment (include_bytes! is align 1, parsing needs align 8)
        let init_data = alloc::vec::Vec::from(INIT_ELF);
        match task::spawn_from_mem(&init_data, "init", types::CellId(1), alloc::vec![]) {
            Ok((init_tid, _load_base)) => {
                log_info("Successfully spawned init");
                // init is the ROOT AUTHORITY (P2 monotonic-downgrade): grant the
                // FULL capability set directly here. init is spawned via
                // spawn_from_mem (NOT spawn_from_path), so its manifest is never
                // read — this direct TCB write is the only place its caps come
                // from. init then delegates subsets to vfs/net/shell/... via the
                // SpawnFromPath syscall, where each child is intersected against
                // init's caps. Escalation-oracle bound: init's spawn targets MUST
                // remain compile-time constants (no data-derived paths).
                if let Some(sched) = task::SCHEDULER.lock().as_mut() {
                    if let Some(t) = sched.tasks.get_mut(&init_tid) {
                        task::cap::CapSet::ALL.apply_to(t);
                    }
                }
                log_info("init granted root authority (CapSet::ALL)");
            }
            Err(_e) => log_info("Failed to spawn init"),
        }
    }

    // Ring-3 smoke test: spawn a minimal U-mode task that logs and exits.
    // RISC-V only — task writes RISC-V machine code directly.
    // Expected serial output: "Hi from U-mode!\n" followed by task exit.
    #[cfg(target_arch = "riscv64")]
    match task::user_hello::spawn() {
        Ok(tid) => {
            puts("[task] spawning user_hello at ");
            // Print tid as decimal (max 20 digits for usize)
            let mut buf = [0u8; 20];
            let mut n = tid;
            let mut i = 20usize;
            if n == 0 { i -= 1; buf[i] = b'0'; } else {
                while n > 0 { i -= 1; buf[i] = b'0' + (n % 10) as u8; n /= 10; }
            }
            let _ = core::str::from_utf8(&buf[i..]).map(|s| puts(s));
            puts("\n");
            let _ = tid; // suppress unused warning
        }
        Err(_) => log_info("[task] user_hello spawn failed"),
    }

    log_info("Kernel initialization complete. Entering idle loop.");

    // 9. Start multitasking
    log_info("Starting scheduler...");

    // Quiet the shared console for interactive use. Kernel bring-up is done; the
    // remaining Info chatter is per-spawn noise ([loader] SpawnFromPath, Spawn:,
    // ELF LOAD) that floods the UART and buries the shell prompt. WARN/ERROR still
    // surface real problems. Raise back to Info when debugging the spawn path.
    log::set_max_level(log::LevelFilter::Warn);

    // Enable interrupts before entering the idle loop.
    // RISC-V: set SPP=1 and SIE=1 in sstatus (0x102).
    // AArch64: clear DAIF.I bit to unmask IRQs.
    // x86_64: STI via ARCH.enable_interrupts().
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    unsafe {
        // SAFETY: csrs sstatus SPP|SIE from S-mode — standard interrupt enable.
        core::arch::asm!("csrs sstatus, {0}", in(reg) 0x102usize);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("msr daifclr, #2", options(nomem, nostack));
    }
    // x86_64, x86_32, AArch32: use the Arch trait's enable_interrupts().
    #[cfg(any(target_arch = "x86_64", target_arch = "x86", target_arch = "arm"))]
    crate::hal::ARCH.enable_interrupts();

    loop {
        crate::task::yield_cpu();
        crate::hal::ARCH.wait_for_interrupt();
    }
}

/// Panic handler.
///
/// If the panic occurs while a Cell is running (`CURRENT_CELL_ID != 0`),
/// kills the Cell instead of halting the system (e.g. OOM after QuotaAlloc
/// returns null → Cell's alloc panics → we kill it, kernel continues).
/// True kernel panics (cell_id == 0) halt as before.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let cell_id = task::hart_local::current_cell_id();

    if cell_id != 0 {
        // Cell OOM/panic — kill the Cell, kernel survives. Print the panic
        // FIRST: this path used to swallow the message entirely, leaving only
        // a meaningless "scause=0x0 sepc=0x0" fault line to debug from.
        {
            #[inline(always)]
            fn cell_panic_putchar(c: u8) {
                #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
                { let _ = crate::hal::sbi::console_putchar(c); }
                #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
                { crate::hal::uart_pl011::putchar(c); }
                #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
                { crate::hal::uart_16550::putchar(c); }
            }
            struct CellPanicWriter;
            impl core::fmt::Write for CellPanicWriter {
                fn write_str(&mut self, s: &str) -> core::fmt::Result {
                    for c in s.bytes() { cell_panic_putchar(c); }
                    Ok(())
                }
            }
            use core::fmt::Write;
            let _ = write!(CellPanicWriter, "\n[panic-in-cell {}] {}\n", cell_id, info);
        }
        // SAFETY: panic context, interrupts disabled (abort mode), single-hart.
        task::terminate_current_cell_on_fault(0, 0, 0);
        // terminate_current_cell_on_fault calls yield_cpu() which switches away.
        // In abort mode we never return here, but placate the compiler:
        #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
        loop { unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); } }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
        loop { unsafe { core::arch::asm!("wfi"); } }
    }

    // True kernel panic: print diagnostics and halt.
    #[inline(always)]
    fn panic_putchar(c: u8) {
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        { let _ = crate::hal::sbi::console_putchar(c); }
        #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
        { crate::hal::uart_pl011::putchar(c); }
        #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
        { crate::hal::uart_16550::putchar(c); }
    }
    let puts = |s: &str| { for c in s.bytes() { panic_putchar(c); } };
    puts("\n[KERNEL PANIC] ");
    puts("Critical failure.\n");
    use core::fmt::Write;
    struct PanicWriter;
    impl core::fmt::Write for PanicWriter {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for c in s.bytes() { panic_putchar(c); }
            Ok(())
        }
    }
    let _ = write!(PanicWriter, "{}\n", info);

    // Reboot or spin: RISC-V uses SBI SRST; ARM64 / x86_64 spin.
    puts("[KERNEL PANIC] halting...\n");
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    crate::hal::sbi::system_reset(crate::hal::sbi::SBI_RESET_COLD_REBOOT, 0);
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    loop { unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); } }
    // Fallback halt for all non-x86 arches (including riscv — unreachable after system_reset).
    #[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
    loop { unsafe { core::arch::asm!("wfi", options(nomem, nostack)); } }
}
