// SPDX-License-Identifier: MPL-2.0
//! ViOS Kernel - Entry point

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]

extern crate alloc;

use core::panic::PanicInfo;

// Core kernel modules
pub mod boot;
pub mod memory;
pub mod cell;
pub mod loader;
pub mod fs; // Filesystem
pub mod task;      // Renamed from 'process'
// pub mod arch; // Moved to HAL
pub extern crate hal; // HAL (Architecture specific)
use hal::Arch;
use boot::BootInfo;
use api::block::ViBlockDevice;


// Internal utilities
mod sync;

// Re-export types for convenience
pub use types::*;

// Embed Init Binary
#[repr(align(4096))]
static INIT_ELF: &[u8] = include_bytes!("embedded/init");

/// Kernel entry point called from HAL boot code
#[no_mangle]
pub extern "C" fn kmain(hartid: usize, dtb: usize) -> ! {
    // 0. Initialize UART immediately for early logging
    task::drivers::uart::init();
    
    // 1. Initialize HAL (Architecture specific) - Early Trap Setup
    hal::ARCH.init();
    
    // Manual puts helper for debugging
    let puts = |s: &str| {
        for c in s.bytes() {
            crate::hal::sbi::console_putchar(c);
        }
    };
    puts("VIOS K MAIN ENTRY\n");

    log::info!("Kernel started (Hart: {}, DTB: 0x{:X})", hartid, dtb);

    // Parse bootloader information
    let boot_info_result = boot::parse_bootloader_info();
    
    // Check if Limine failed, if so, use fallback (SimpleBootInfo)
    let boot_info: &dyn BootInfo = match &boot_info_result {
        Ok(info) => info,
        Err(_) => {
            log::warn!("Limine not found, using QEMU/OpenSBI fallback");
            // Use fallback static instance (defined in boot.rs or created here)
            // For now, let's just use the fallback function we will create
            unsafe { &boot::FALLBACK_BOOT_INFO }
        }
    };
    // Force type validity (we can't return reference to local without more work, 
    // so we assume FALLBACK_BOOT_INFO is static)
    
    // Initialize kernel subsystems
    
    // 1. Memory Management
    // Get memory map from Boot Info (Converted to ViOS format)
    let mmap_entries = boot_info.memory_map();
    
    // Initialize frame allocator with the largest usable region
    let mut frame_allocator = memory::frame::FrameAllocator::new_from_map(mmap_entries);
    
    // 2. Frame Allocator (Physical Memory)
    // The local `frame_allocator` is moved into the global static.
    // A mutable reference to the global static will be used for paging setup.
    unsafe {
        core::ptr::write(&mut *memory::frame::FRAME_ALLOCATOR.lock(), Some(frame_allocator));
    }
    log::info!("Frame allocator initialized");

    // 3. Paging (Virtual Memory) Setup
    log::info!("Initializing paging...");
    // Get a mutable reference to the global frame allocator for paging initialization.
    let mut locked_frame_allocator = memory::frame::FRAME_ALLOCATOR.lock();
    let root_table_phys = memory::paging::init_kernel_paging(
        locked_frame_allocator.as_mut().expect("Frame allocator not initialized"),
        mmap_entries
    ).expect("Failed to initialize paging");
    drop(locked_frame_allocator);
    log::info!("Paging initialized at 0x{:X}", root_table_phys);

    // Activate Paging (SV39)
    log::info!("Activating paging...");
    unsafe {
        memory::paging::activate_paging(root_table_phys);
    }
    log::info!("Paging activated");

    // 4. Heap Allocator (Global) - MUST be after paging but before any allocations
    // Allocate 16MB for heap (4096 pages)
    let heap_start = {
        let mut allocator_guard = memory::frame::FRAME_ALLOCATOR.lock();
        let allocator = allocator_guard.as_mut().expect("Frame allocator not initialized");
        let start = allocator.allocate_frame().expect("OOM: Heap start");
        for _ in 0..4095 {
            allocator.allocate_frame().expect("OOM: Heap continuation");
        }
        start
    };
    let heap_size = 4096 * 4096; // 16MB
    unsafe {
        memory::heap::init_heap(heap_start, heap_size);
    }
    log::info!("Heap initialized: 16MB at 0x{:X}", heap_start);

    // Test Heap
    let mut vec = alloc::vec::Vec::new();
    vec.push(1);
    vec.push(2);
    vec.push(3);
    log::info!("Heap test passed: vec = {:?}", vec);

    // 5. Hardware Abstraction Layer (HAL) Initialization
    // Already initialized at step 1 for trap handling
    log::info!("HAL initialized");

    // 6. Logger & Drivers & FS
    task::drivers::uart::init();
    task::drivers::init();

    fs::init(); // Re-enabled with RAM disk
    log::info!("Kernel subsystems initialized successfully.");

    // 7. Initialize Scheduler
    log::info!("Initializing scheduler...");
    task::init();
    log::info!("Scheduler initialized");

    // 8. Spawn Embedded Init
    log::info!("Spawning Embedded Init...");
    match task::spawn_from_mem(INIT_ELF, "init", types::CellId(1), alloc::vec![]) {
        Ok(tid) => log::info!("Successfully spawned init (TID: {})", tid),
        Err(e) => log::error!("Failed to spawn init: {:?}", e),
    }
    
    log::info!("Kernel initialization complete. Entering idle loop.");
    
    // 9. Start multitasking
    log::info!("Starting scheduler...");
    
    // Ensure SPP bit is set in sstatus so that context switch saves it as Supervisor Mode.
    // DISABLE Interrupts (0x100) to avoid potential trap handler bugs for now.
    unsafe {
        core::arch::asm!("csrs sstatus, {0}", in(reg) 0x100);
    }

    loop {
        if !crate::task::has_ready_tasks() {
             log::info!("kmain: idle loop (no tasks)");
        }
        crate::task::yield_cpu();
        // Use global HAL instance
        crate::hal::ARCH.wait_for_interrupt();
    }
}

/// Panic handler
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log::error!("{}", info);
    loop {
        hal::ARCH.wait_for_interrupt();
    }
}
