#![no_std]
#![no_main]
// #[no_mangle] on main() requires allowing unsafe_attr — cannot use forbid(unsafe_code).
// All peripheral-access code is unsafe-free (uses MmioRegion abstraction).

extern crate alloc;

use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use driver_serial::Pl011Uart;
use hal_gpio::{Edge, PinDir, ViGpio};
use types::ViError;
use hal_uart::SerialPort;
use ostd::io::println;
use ostd::syscall::{sys_get_wall_secs, sys_recv_timeout, sys_spawn_pinned, SyscallResult};

declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = true);

const SELF_PATH: &str = "/bin/periph-demo";
const POLL_PRIORITY: u8 = 200;

#[no_mangle]
pub fn main() {
    // ── RTC test ──────────────────────────────────────────────────────────────
    // sys_get_wall_secs() calls GetTime op=3 → Goldfish RTC epoch seconds.
    // Passes if the RTC returned a plausible year (> 2020-01-01 = 1577836800).
    {
        let epoch_secs = sys_get_wall_secs();
        let year_approx = 1970u64 + epoch_secs / 31_557_600; // ~365.25 days
        let msg = if epoch_secs > 1_577_836_800 {
            alloc::format!(
                "[periph-demo] RTC OK: epoch={} (~year {})",
                epoch_secs, year_approx
            )
        } else {
            alloc::format!(
                "[periph-demo] RTC FAIL: epoch={} (expected >1577836800; RTC not initialized?)",
                epoch_secs
            )
        };
        println(&msg);
    }

    // ── GPIO demo ─────────────────────────────────────────────────────────────
    // robot-demo (supervised, spawned before us) holds PL061 for its 5-cycle
    // sensor→actuator loop plus MQTT timeouts per cycle.  Retry only on
    // AlreadyExists (resource busy) — fail immediately on PermissionDenied or
    // other errors (non-aarch64 target or manifest mismatch).
    let gpio_result = {
        let mut res = Pl061Gpio::open();
        let mut tries = 0u8;
        while matches!(res, Err(ViError::AlreadyExists)) && tries < 150 {
            let mut buf = [0u8; 8];
            let _ = sys_recv_timeout(0, &mut buf, 10); // ~100 ms per attempt
            tries += 1;
            res = Pl061Gpio::open();
        }
        res
    };
    match gpio_result {
        Ok(mut gpio) => {
            println("[periph-demo] GPIO PL061 opened");

            // Basic output test: write HIGH → verify HIGH, write LOW → verify LOW.
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

            // ── GPIO IRQ self-test (output-pin loopback) ───────────────────
            // PL061 edge detection monitors pin state. For output pins the output
            // value IS the pin state, so writing 0→1 should set GPIORIS bit 0
            // and (with GPIOIE set) GPIOMIS bit 0 and fire GIC ID 39.
            //
            // The kernel vi_gpio_notify_irq IPC fires immediately when the GPIO
            // write triggers the edge (cell not yet in Recv state → IPC dropped).
            // But GPIOMIS is a sticky HW register — it persists until we clear it,
            // so we can read it directly to confirm the edge was detected.
            let _ = gpio.enable_edge_irq(0, Edge::Rising); // enable on pin 0
            let _ = gpio.write_pin(0, true);               // 0 → 1 : rising edge
            // At this point vi_gpio_notify_irq was called by the kernel (see kernel
            // log "[gpio-irq] IRQ fired") even though the IPC was dropped.
            let mis = gpio.read_mis().unwrap_or(0xFF);
            let _ = gpio.clear_irq(mis);
            let _ = gpio.disable_irq(0);
            let _ = gpio.write_pin(0, false); // restore LOW
            {
                let msg = if mis & 1 != 0 {
                    alloc::format!(
                        "[periph-demo] GPIO IRQ OK: GPIOMIS=0x{:02x} (edge detected on pin 0)",
                        mis
                    )
                } else {
                    alloc::format!(
                        "[periph-demo] GPIO IRQ: GPIOMIS=0x{:02x} \
                         (0=edge not detected; check QEMU PL061 output-loopback behavior)",
                        mis
                    )
                };
                println(&msg);
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
    println("[periph-demo] spawning pinned-poll cell on hart 0");
    match sys_spawn_pinned(SELF_PATH, POLL_PRIORITY, 0) {
        SyscallResult::Ok(tid) => {
            let tid_str = alloc::format!("[periph-demo] pinned cell tid={}", tid);
            println(&tid_str);
        }
        SyscallResult::Err(_) => println("[periph-demo] spawn_pinned unavailable"),
    }
}
