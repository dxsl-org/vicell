#![no_std]
#![no_main]
// #[no_mangle] on main() requires allowing unsafe_attr — cannot use forbid(unsafe_code).
// All peripheral-access code is unsafe-free (uses MmioRegion abstraction).

extern crate alloc;

use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use driver_serial::Pl011Uart;
use hal_gpio::{PinDir, ViGpio};
use hal_uart::{SerialPort, ViUart};
use ostd::io::println;
use ostd::syscall::{sys_spawn_pinned, SyscallResult};

declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = true);

const SELF_PATH: &str = "/bin/periph-demo";
const POLL_PRIORITY: u8 = 200;

#[no_mangle]
pub fn main() {
    // ── GPIO demo ─────────────────────────────────────────────────────────────
    match Pl061Gpio::open() {
        Ok(mut gpio) => {
            println("[periph-demo] GPIO PL061 opened");
            let _ = gpio.set_direction(0, PinDir::Output);
            let _ = gpio.write_pin(0, true);
            match gpio.read_pin(0) {
                Ok(true)  => println("[periph-demo] GPIO pin 0: HIGH OK"),
                Ok(false) => println("[periph-demo] GPIO pin 0: LOW (unexpected)"),
                Err(_)    => println("[periph-demo] GPIO read_pin error"),
            }
            let _ = gpio.write_pin(0, false);
            match gpio.read_pin(0) {
                Ok(false) => println("[periph-demo] GPIO pin 0: LOW OK"),
                Ok(true)  => println("[periph-demo] GPIO pin 0: HIGH (unexpected)"),
                Err(_)    => println("[periph-demo] GPIO read_pin error"),
            }
        }
        Err(_) => println("[periph-demo] GPIO not available (non-aarch64 target)"),
    }

    // ── UART demo ─────────────────────────────────────────────────────────────
    match Pl011Uart::open() {
        Ok(mut uart) => {
            println("[periph-demo] UART PL011 opened");
            let _ = uart.init();
            for b in b"[periph-demo] Hello from PL011\r\n" {
                let _ = uart.send(*b);
            }
            println("[periph-demo] UART TX done");
        }
        Err(_) => println("[periph-demo] UART not available (non-aarch64 target)"),
    }

    // ── Pinned-poll RT loop ───────────────────────────────────────────────────
    // Spawn a copy of this cell pinned to hart 0 for hard-RT GPIO polling.
    // The spawned copy also runs main() and exits after the demo above.
    println("[periph-demo] spawning pinned-poll cell on hart 0");
    match sys_spawn_pinned(SELF_PATH, POLL_PRIORITY, 0) {
        SyscallResult::Ok(tid) => {
            let tid_str = alloc::format!("[periph-demo] pinned cell tid={}", tid);
            println(&tid_str);
        }
        SyscallResult::Err(_) => println("[periph-demo] spawn_pinned unavailable"),
    }
}
