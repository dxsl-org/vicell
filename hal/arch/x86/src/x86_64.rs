use hal_arch_trait::{Arch, BootController};
use types::HalResult;

pub struct X86_64Arch;

// TODO: Implement Arch trait for X86_64
pub type PlatformArch = X86_64Arch;
pub static ARCH: PlatformArch = X86_64Arch;

impl Arch for X86_64Arch {
    type Context = usize; // TODO: Define real context

    fn init(&self) {
        // TODO: Initialize x86_64
    }
    
    unsafe fn switch_context(&self, _old: *mut Self::Context, _new: *const Self::Context) {
        // TODO: Implement context switching
        unimplemented!("x86_64 context switch not implemented");
    }
    
    fn enable_interrupts(&self) {
        // TODO
    }
    
    fn disable_interrupts(&self) {
        // TODO
    }
    
    fn wait_for_interrupt(&self) {
        // TODO
    }

    fn interrupts_enabled(&self) -> bool {
        // TODO
        false
    }
}
