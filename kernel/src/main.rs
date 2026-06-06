// SPDX-License-Identifier: MPL-2.0
//! ViCell Kernel - Entry point

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::panic::PanicInfo;

// Core kernel modules
pub mod audit;
pub mod boot;
pub mod cell;
pub mod fast_ipc; // Kernel-owned fast-IPC dispatch table (canonical instance)
pub mod fs; // Filesystem
pub mod loader;
pub mod memory;
pub mod snapshot;
pub mod task; // Renamed from 'process'
              // pub mod arch; // Moved to HAL
pub extern crate hal; // HAL (Architecture specific)
// // use api::block::ViBlockDevice;
use api::posix::_putchar;
// use api::posix::puts;
use boot::BootInfo;
use hal::Arch;

// Internal utilities
mod sync;

// Re-export types for convenience
pub use types::*;

// Embed Init Binary (stripped by build.rs, served from OUT_DIR)
static INIT_ELF: &[u8] = include_bytes!(concat!(env!("EMBEDDED_OUT_DIR"), "/init"));

/// Kernel entry point called from HAL boot code
#[no_mangle]
pub extern "C" fn kmain(hartid: usize, dtb: usize) -> ! {
    let _hartid = hartid;
    let _dtb = dtb;
    // 0. Initialize UART immediately for early logging
    task::drivers::uart::init();

    // 1. Initialize HAL (Architecture specific) - Early Trap Setup
    hal::ARCH.init();

    // Define puts helper using imported _putchar
    let puts = |s: &str| {
        for c in s.bytes() {
            unsafe { _putchar(c as u8); }
        }
    };

    // Restore log_info helper
    let log_info = |s: &str| {
        puts("[INFO] ");
        puts(s);
        puts("\n");
    };

    // Stable banner — CI greps for this exact string.
    puts("[ViCell] kernel boot v");
    puts(env!("CARGO_PKG_VERSION"));
    puts("\n");
    puts("Kernel started (Hart: 0, DTB: ...)\n");

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
    // Get memory map from Boot Info (Converted to ViCell format)
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
    log_info("Initializing paging...");
    // Get a mutable reference to the global frame allocator for paging initialization.
    let mut locked_frame_allocator = memory::frame::FRAME_ALLOCATOR.lock();
    puts("TRACE: Calling init_kernel_paging\n");
    let root_table_phys = memory::paging::init_kernel_paging(
        locked_frame_allocator
            .as_mut()
            .expect("Frame allocator not initialized"),
        mmap_entries,
    )
    .expect("Failed to initialize paging");
    puts("TRACE: init_kernel_paging returned\n");
    drop(locked_frame_allocator);
    log_info("Paging initialized");

    // Activate Paging (SV39)
    log_info("Activating paging...");
    unsafe {
        memory::paging::activate_paging(root_table_phys);
    }
    puts("TRACE: Paging activated (satp set)\n");
    log_info("Paging activated");

    // 4. Heap Allocator (Global) - MUST be after paging but before any allocations
    puts("TRACE: Allocating heap frames\n");
    // Allocate 16 MB for the kernel heap.
    // Rationale: kernel_fs.img is ~4 MB so the heap needs to be > 4 MB.
    // 16 MB gives plenty of room for the RAM-disk copy + all kernel data structures
    // (BTreeMaps for tasks, capabilities, page table frames, etc.).
    // 32 MB = 8 192 frames of 4 KB each.  Sized to hold simultaneously:
    //   - the embedded RAM disk copy (kernel_fs.img, ~8 MB with lua + python)
    //   - the VirtIO GPU framebuffer (1280×800×4 ≈ 4 MB)
    //   - cell ELFs loaded via SpawnFromPath + kernel structures
    // 16 MB was too tight once lua + python entered kernel_fs.img (GPU
    // framebuffer alloc OOM'd, hanging the boot at the GPU probe).
    // 128 MB QEMU instances have ample room for a 32 MB heap.
    const HEAP_FRAMES: usize = 8_192;
    let heap_start = {
        let mut allocator_guard = memory::frame::FRAME_ALLOCATOR.lock();
        let allocator = allocator_guard
            .as_mut()
            .expect("Frame allocator not initialized");
        let start = allocator.allocate_frame().expect("OOM: Heap start");
        for _ in 1..HEAP_FRAMES {
            allocator.allocate_frame().expect("OOM: Heap continuation");
        }
        start
    };
    puts("TRACE: frames allocated, calling init_heap\n");
    let heap_size = HEAP_FRAMES * 4096; // 32 MB — matches frames above
    unsafe {
        memory::heap::init_heap(heap_start, heap_size);
    }
    puts("TRACE: init_heap done\n");
    log_info("Heap initialized");

    // Initialize RT TLSF pool for RealTime cell stack allocation.
    memory::rt_heap::init();
    log_info("RT heap initialized");

    // Test Heap
    puts("TRACE: Testing Vec\n");
    let mut vec = alloc::vec::Vec::new();
    vec.push(1);
    vec.push(2);
    vec.push(3);
    puts("TRACE: Vec test passed\n");
    // log::info!("Heap test passed: vec = {:?}", vec);
    log_info("Heap test passed");

    // 5. Hardware Abstraction Layer (HAL) Initialization
    // Already initialized at step 1 for trap handling.
    // Initialize PLIC for external interrupts.
    puts("TRACE: init PLIC\n");
    #[cfg(target_arch = "riscv64")]
    crate::hal::common::plic::init();
    puts("TRACE: PLIC done\n");

    log_info("HAL initialized (PLIC enabled)");

    // 6. Logger & Drivers & FS
    puts("TRACE: init drivers::uart\n");
    task::drivers::uart::init();
    task::drivers::uart::init_input(); // Initialize RX buffer
    puts("TRACE: init drivers\n");
    task::drivers::init();
    puts("TRACE: drivers done\n");

    // Attempt warm boot from snapshot before any cell initialization.
    // try_restore() replays allocated physical frames (including kernel .bss/.data
    // globals like SCHEDULER) from disk, then yields into the restored task set.
    // Returns false (continues cold boot) if no valid snapshot is found.
    if snapshot::try_restore() {
        // try_restore() called yield_cpu() and should not return in a successful
        // warm boot.  If we reach here, fall through to cold boot as a safety net.
        log::warn!("[boot] snapshot restore returned unexpectedly → cold boot");
    }

    // Probe the cell bootstrap table so SpawnFromPath works during init.
    // Failure is non-fatal: init will log warnings if it cannot spawn cells.
    match crate::loader::early::EarlyLoader::probe() {
        Ok(()) => puts("[loader] cell bootstrap table loaded\n"),
        Err(_) => puts("[loader] WARN: cell table not found — disk image may lack bootstrap section\n"),
    }

    fs::init(); // Re-enabled with RAM disk

    // Phase 20: verify the hot-migration state-transfer primitive round-trips.
    crate::cell::state_stash::self_test();

    log_info("Kernel subsystems initialized successfully.");

    // 7. Initialize Scheduler
    log_info("Initializing scheduler...");
    task::init();
    log_info("Scheduler initialized");

    // 8. Spawn Embedded Init
    log_info("Spawning Embedded Init...");
    
    // Enable SUM (Supervisor User Memory access) bit in sstatus (bit 18 = 0x40000)
    // This allows the Kernel (S-mode) to access User (U-mode) pages, which is required
    // when writing the initial stack for the new process.
    unsafe {
        core::arch::asm!("csrs sstatus, {0}", in(reg) 0x40000);
    }
    
    // Copy to Vec to ensure alignment (include_bytes! is align 1, parsing needs align 8)
    let init_data = alloc::vec::Vec::from(INIT_ELF);
    match task::spawn_from_mem(&init_data, "init", types::CellId(1), alloc::vec![]) {
        Ok(init_tid) => {
            log_info("Successfully spawned init");
            // Grant SpawnCap to init: it uses sys_spawn_from_path (a syscall) to
            // boot vfs/config/shell. Without SpawnCap the syscall returns PermissionDenied.
            if let Some(sched) = task::SCHEDULER.lock().as_mut() {
                if let Some(t) = sched.tasks.get_mut(&init_tid) {
                    t.spawn_cap = Some(task::cap::SpawnCap::new());
                }
            }
        }
        Err(_e) => log_info("Failed to spawn init"),
    }

    // Ring-3 smoke test: spawn a minimal U-mode task that logs and exits.
    // Expected serial output: "Hi from U-mode!\n" followed by task exit.
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

    // Ensure SPP bit is set in sstatus so that context switch saves it as Supervisor Mode.
    // ENABLE Interrupts (SIE=1) now that we are ready to handle them!
    // We used to disable (0x100) but now we want to test PLIC.
    // sstatus = 0x102 (SPP=1, SIE=1)
    unsafe {
        core::arch::asm!("csrs sstatus, {0}", in(reg) 0x102);
    }

    loop {
        if !crate::task::has_ready_tasks() {
            // log::info!("kmain: idle loop (no tasks)");
        }
        crate::task::yield_cpu();
        // Use global HAL instance
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
    let cell_id = task::scheduler::CURRENT_CELL_ID.load(
        core::sync::atomic::Ordering::Relaxed
    );

    if cell_id != 0 {
        // Cell OOM/panic — kill the Cell, kernel survives.
        // SAFETY: panic context, interrupts disabled (abort mode), single-hart.
        task::terminate_current_cell_on_fault(0, 0);
        // terminate_current_cell_on_fault calls yield_cpu() which switches away.
        // In abort mode we never return here, but placate the compiler:
        loop { unsafe { core::arch::asm!("wfi"); } }
    }

    // True kernel panic: print diagnostics and halt.
    let puts = |s: &str| {
        for c in s.bytes() {
            let _ = crate::hal::sbi::console_putchar(c);
        }
    };
    puts("\n[KERNEL PANIC] ");
    puts("Critical failure.\n");
    use core::fmt::Write;
    struct PanicWriter;
    impl core::fmt::Write for PanicWriter {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for c in s.bytes() { let _ = crate::hal::sbi::console_putchar(c); }
            Ok(())
        }
    }
    let _ = write!(PanicWriter, "{}\n", info);

    // A dead kernel must come back up, not freeze — a frozen robot is unrecoverable
    // without physical intervention. Request a cold reboot via SBI SRST. If the
    // firmware lacks SRST, system_reset returns and we fall back to halting.
    puts("[KERNEL PANIC] rebooting...\n");
    crate::hal::sbi::system_reset(crate::hal::sbi::SBI_RESET_COLD_REBOOT, 0);
    loop { hal::ARCH.wait_for_interrupt(); }
}
