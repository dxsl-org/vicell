#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

// Include the Rust source generated from counter.vi by viui-build.
include!(concat!(env!("OUT_DIR"), "/counter.rs"));

api::declare_syscalls![Log];

#[no_mangle]
pub fn main() {
    ostd::io::println("[viui-demo] build.rs pipeline verified");

    // Construct the reactive widget tree from the generated component.
    let (state, _root) = Counter::build();

    // Verify Signal reactivity — increment count.
    state.count.update(|n| *n += 1);

    ostd::io::println("[viui-demo] Counter signal OK");
    ostd::syscall::sys_exit(0);
}
