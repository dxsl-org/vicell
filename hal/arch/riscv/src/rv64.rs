pub mod boot;

// Re-export common modules for convenience or trait impls
pub use crate::common::sbi;
pub use crate::common::timer;
pub use crate::common::uart_ns16550a as uart;

mod asm;
pub mod context;
pub mod paging;
pub mod trap;
pub use paging::*;

pub mod arch {
    pub use crate::rv64::context::*;
    pub use crate::rv64::trap::*;

    extern "C" {
        pub fn thread_trampoline();
    }
}

pub use hal_arch_trait::*;

pub use types::*;

/// RISC-V architecture implementation.
pub struct RiscVArch;

pub type PlatformArch = RiscVArch;

pub static ARCH: PlatformArch = RiscVArch;

impl Arch for RiscVArch {
    type Context = context::Context;

    fn init(&self) {
        // Initialize trap handling (set stvec)
        trap::init();

        // Enable S-mode software interrupt (SSIP) so RT cells can trigger
        // zero-latency preemption via `csrsi sip, 0x2` from kernel code.
        // SAFETY: csrsi on sie sets only the SSIE bit (bit 1); safe from S-mode.
        #[cfg(target_arch = "riscv64")]
        unsafe { core::arch::asm!("csrsi sie, 0x2"); }
    }

    unsafe fn switch_context(&self, old: *mut Self::Context, new: *const Self::Context) {
        context::Context::switch(old, new);
    }

    fn enable_interrupts(&self) {
        unsafe {
            riscv::register::sstatus::set_sie();
        }
    }

    fn disable_interrupts(&self) {
        unsafe {
            riscv::register::sstatus::clear_sie();
        }
    }

    fn wait_for_interrupt(&self) {
        unsafe {
            riscv::asm::wfi();
        }
    }

    fn interrupts_enabled(&self) -> bool {
        riscv::register::sstatus::read().sie()
    }
}
