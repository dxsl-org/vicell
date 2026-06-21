#![no_std]
#![no_main]

use ostd::app::{AppContext, AppEvent};
use ostd::io::println;

// Zero boilerplate — manifest, syscall allowlist, and main() generated automatically.
ostd::app_entry!(handler = hello_handler);

fn hello_handler(_ctx: &mut AppContext, event: AppEvent) {
    match event {
        AppEvent::Init => println("Hello from ViCell!"),
        AppEvent::Shutdown | AppEvent::ShutdownWith { .. } => {
            ostd::syscall::sys_exit(0);
        }
        _ => {}
    }
}
