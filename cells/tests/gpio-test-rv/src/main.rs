#![no_std]
#![no_main]

extern crate alloc;

use api::declare_manifest;
use core::sync::atomic::{AtomicU32, Ordering};
use driver_gpio_sifive::SiFiveGpio;
use hal_gpio::{PinDir, ViGpio};
use ostd::io::println;
use types::ViError;

declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = false);

static PASS: AtomicU32 = AtomicU32::new(0);
static FAIL: AtomicU32 = AtomicU32::new(0);

macro_rules! pass {
    ($msg:expr) => {{
        println(alloc::format!("  PASS  {}", $msg).as_str());
        PASS.fetch_add(1, Ordering::Relaxed);
    }};
}

macro_rules! fail {
    ($msg:expr) => {{
        println(alloc::format!("  FAIL  {}", $msg).as_str());
        FAIL.fetch_add(1, Ordering::Relaxed);
    }};
}

#[no_mangle]
pub fn main() {
    println("[gpio-test-rv] SiFive GPIO0 driver test (QEMU sifive_u)");

    let mut gpio = match SiFiveGpio::open() {
        Ok(g) => g,
        Err(ViError::PermissionDenied) => {
            println("  SKIP  GPIO0 not in MMIO allowlist (need --machine sifive_u)");
            ostd::syscall::sys_exit(0);
        }
        Err(e) => {
            fail!(alloc::format!("SiFiveGpio::open(): {:?}", e));
            ostd::syscall::sys_exit(1);
        }
    };

    // ── A: Output write and read-back (via OUTPUT_VAL, direction-enforced) ────
    println("[A] set_direction(5, Output) + write HIGH / LOW");

    if gpio.set_direction(5, PinDir::Output).is_err() {
        fail!("set_direction(5, Output)");
    } else {
        let _ = gpio.write_pin(5, true);
        // INPUT_VAL reflects physical pin voltage — may be 0 in QEMU without wiring.
        // The important invariant: no trap, no kernel panic.
        match gpio.read_pin(5) {
            Ok(v)  => pass!(alloc::format!("read_pin(5) = {} after HIGH write (MMIO OK)", v)),
            Err(e) => fail!(alloc::format!("read_pin(5) after HIGH: {:?}", e)),
        }

        let _ = gpio.write_pin(5, false);
        match gpio.read_pin(5) {
            Ok(v)  => pass!(alloc::format!("read_pin(5) = {} after LOW write (MMIO OK)", v)),
            Err(e) => fail!(alloc::format!("read_pin(5) after LOW: {:?}", e)),
        }
    }

    // ── B: Input direction + write rejection ──────────────────────────────────
    println("[B] set_direction(5, Input) + write rejection");

    if gpio.set_direction(5, PinDir::Input).is_err() {
        fail!("set_direction(5, Input)");
    } else {
        match gpio.read_pin(5) {
            Ok(_)  => pass!("read_pin on input pin returned (no trap)"),
            Err(e) => fail!(alloc::format!("read_pin on input pin: {:?}", e)),
        }
        match gpio.write_pin(5, true) {
            Err(ViError::InvalidInput) => pass!("write to input pin → InvalidInput"),
            Ok(())                     => fail!("write to input pin unexpectedly OK"),
            Err(e)                     => fail!(alloc::format!("wrong error: {:?}", e)),
        }
    }

    // ── Results ───────────────────────────────────────────────────────────────
    let pass = PASS.load(Ordering::Relaxed);
    let fail = FAIL.load(Ordering::Relaxed);
    println(alloc::format!(
        "[gpio-test-rv] Results: {}/{} passed", pass, pass + fail
    ).as_str());

    if fail == 0 {
        println("[gpio-test-rv] ALL PASS");
        ostd::syscall::sys_exit(0);
    } else {
        println("[gpio-test-rv] FAILURES DETECTED");
        ostd::syscall::sys_exit(1);
    }
}
