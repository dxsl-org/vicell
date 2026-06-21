#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use ostd::app::{AppContext, AppEvent};
use ostd::io::println;

// Declare caps + generate manifest, syscall allowlist, and main() boilerplate.
ostd::app_entry!(handler = demo_handler);

fn demo_handler(ctx: &mut AppContext, event: AppEvent) {
    match event {
        AppEvent::Init => {
            println("[sdk-demo] starting — SDK L1 demo");

            // Service discovery via ergonomic client (no manual VfsRef construction)
            match ctx.vfs().stat("/") {
                Ok((size, is_dir)) => {
                    let s = format!("[sdk-demo] VFS stat('/') ok — size={size} is_dir={is_dir}");
                    println(&s);
                }
                Err(_) => {
                    // VFS may not be registered on minimal test boots — expected.
                    println("[sdk-demo] VFS not available (service not registered)");
                }
            }

            println("[sdk-demo] init complete — waiting for messages");
        }

        AppEvent::Message { sender_tid, data } => {
            let reply = format!(
                "[sdk-demo] echo from tid={sender_tid} ({} bytes)",
                data.len()
            );
            println(&reply);
            ctx.send_msg(sender_tid, &data).ok();
        }

        AppEvent::RawMessage { sender_tid, data } => {
            let s = format!(
                "[sdk-demo] raw msg from tid={sender_tid} ({} bytes) — ignoring",
                data.len()
            );
            println(&s);
        }

        AppEvent::Shutdown => {
            println("[sdk-demo] shutdown — exiting");
            ostd::syscall::sys_exit(0);
        }

        AppEvent::ShutdownWith { reason } => {
            let s = format!("[sdk-demo] shutdown ({reason:?}) — exiting");
            println(&s);
            ostd::syscall::sys_exit(0);
        }

        _ => {}
    }
}
