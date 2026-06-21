#![no_std]
#![no_main]
extern crate ostd;

/// shutdown — halt the system via the kernel's architecture-independent shutdown syscall.
///
/// Invokes syscall 502 (Shutdown) through ostd, which the kernel routes to:
///   - RISC-V: SBI SRST (System Reset Extension) ecall
///   - ARM64:  PSCI SYSTEM_OFF
///   - x86_64: QEMU isa-debug-exit or ACPI S5
/// Never returns if the host accepts the shutdown request.
#[no_mangle]
pub fn main() {
    ostd::io::println("System shutting down...");
    ostd::syscall::sys_shutdown();
}
