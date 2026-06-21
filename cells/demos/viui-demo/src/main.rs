#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

// ── Path 1: build.rs  ────────────────────────────────────────────────────────
// Counter component generated from counter.vi by viui-build → OUT_DIR/counter.rs
include!(concat!(env!("OUT_DIR"), "/counter.rs"));

// ── Path 2: inline proc macro ────────────────────────────────────────────────
// Hello component declared directly; vi_design! runs vi-compiler at compile time.
use viui::vi_design;
vi_design!(r#"
component Hello {
    VerticalLayout {
        padding: 8px;
        Text { text: "Hello, ViUI!"; color: #aaffaa; }
    }
}
"#);

api::declare_syscalls![Log];

// ── Path 3: GpuRenderer<CpuExecutor> type-level proof ────────────────────────
// Prove GpuRenderer<CpuExecutor> satisfies ViRenderer at compile time.
// Cannot call render() here — no real ViSurface in a demo cell.
// Type-level proof is sufficient for P07 architecture validation.
fn _assert_gpu_renderer_api() {
    fn _check<T: viui::renderer::ViRenderer>() {}
    _check::<viui::GpuRenderer<viui::CpuExecutor>>();
}

#[no_mangle]
pub fn main() {
    // build.rs generated component
    let (state, _counter_ui) = Counter::build();
    state.count.update(|n| *n += 1);
    ostd::io::println("[viui-demo] Counter (build.rs) OK");

    // inline proc macro component
    let (_hello_state, _hello_ui) = Hello::build();
    ostd::io::println("[viui-demo] Hello (vi_design!) OK");

    ostd::io::println("[viui-demo] GpuRenderer<CpuExecutor>: ViRenderer OK");
    ostd::syscall::sys_exit(0);
}
