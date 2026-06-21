#![no_std]
#![no_main]

extern crate alloc;

use api::declare_manifest;
use core::sync::atomic::{AtomicU32, Ordering};
use driver_gpio::Pl061Gpio;
use driver_serial::Pl011Uart;
use hal_gpio::{PinDir, ViGpio};
use hal_uart::{SerialPort, ViUart};
use ostd::io::println;
use ostd::mmio::request_region;
use types::ViError;

// Declare gpio + uart caps so the kernel grants MMIO access.
declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = true);

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
    println("[periph-test] ViCell peripheral integration tests");
    println("[periph-test] target: QEMU ARM virt (PL061 GPIO + PL011 UART)");
    println("");

    // Open GPIO once and pass to both GPIO scenarios — MmioRegion has no per-instance
    // Drop/release yet (Law 8 debt, tracked separately), so re-opening the same range
    // within a Cell's lifetime returns AlreadyExists. Reusing one instance avoids this.
    match Pl061Gpio::open() {
        Ok(mut gpio) => {
            scenario_gpio_output_readback(&mut gpio);
            scenario_gpio_direction_input(&mut gpio);
        }
        Err(ViError::PermissionDenied) => {
            println("  SKIP  GPIO not in allowlist (non-aarch64 target)");
        }
        Err(e) => {
            fail!(alloc::format!("GPIO open: {:?}", e));
        }
    }

    scenario_uart_loopback();
    scenario_cap_reject();

    let pass = PASS.load(Ordering::Relaxed);
    let fail = FAIL.load(Ordering::Relaxed);
    let summary = alloc::format!(
        "[periph-test] Results: {}/{} passed", pass, pass + fail
    );
    println(&summary);

    if fail == 0 {
        println("[periph-test] ALL PASS");
        ostd::syscall::sys_exit(0);
    } else {
        println("[periph-test] FAILURES DETECTED");
        ostd::syscall::sys_exit(1);
    }
}

// ── Scenario A: GPIO output write→read-back ───────────────────────────────────
//
// PL061 GPIODATA reflects output register value when pin is configured as output.
// No physical loopback needed — register read-back validates the MMIO path.
fn scenario_gpio_output_readback(gpio: &mut Pl061Gpio) {
    println("[A] GPIO output write→read-back");

    if gpio.set_direction(0, PinDir::Output).is_err() {
        fail!("set_direction(0, Output)");
        return;
    }

    let _ = gpio.write_pin(0, true);
    match gpio.read_pin(0) {
        Ok(true)  => pass!("pin 0 HIGH read-back correct"),
        Ok(false) => fail!("pin 0 HIGH: read-back returned LOW"),
        Err(e)    => fail!(alloc::format!("read_pin after HIGH: {:?}", e)),
    }

    let _ = gpio.write_pin(0, false);
    match gpio.read_pin(0) {
        Ok(false) => pass!("pin 0 LOW read-back correct"),
        Ok(true)  => fail!("pin 0 LOW: read-back returned HIGH"),
        Err(e)    => fail!(alloc::format!("read_pin after LOW: {:?}", e)),
    }
}

// ── Scenario B: GPIO input direction + write rejection ────────────────────────
//
// Set pin 1 as input; QEMU has no external stimulus so reads return 0.
// write_pin on an input pin must return InvalidInput (ViGpio contract).
fn scenario_gpio_direction_input(gpio: &mut Pl061Gpio) {
    println("[B] GPIO input direction + write-to-input rejection");

    if gpio.set_direction(1, PinDir::Input).is_err() {
        fail!("set_direction(1, Input)");
        return;
    }

    match gpio.read_pin(1) {
        Ok(_)  => pass!("read_pin on input pin returned (no crash)"),
        Err(e) => fail!(alloc::format!("read_pin on input pin: {:?}", e)),
    }

    match gpio.write_pin(1, true) {
        Err(ViError::InvalidInput) => pass!("write_pin to input pin → InvalidInput"),
        Ok(())                     => fail!("write_pin to input pin unexpectedly OK"),
        Err(e)                     => fail!(alloc::format!("write_pin to input: wrong error {:?}", e)),
    }
}

// ── Scenario C: UART PL011 loopback via UARTCR.LBE ───────────────────────────
//
// PL011 Loopback Enable (bit 7 of UARTCR) feeds TX data directly into RX FIFO.
// QEMU emulates this synchronously — rx_ready() asserts immediately after send().
fn scenario_uart_loopback() {
    println("[C] UART PL011 TX/RX loopback (LBE)");

    let mut uart = match Pl011Uart::open() {
        Ok(u) => u,
        Err(ViError::PermissionDenied) => {
            println("  SKIP  UART not in allowlist (non-aarch64 target)");
            return;
        }
        Err(e) => {
            fail!(alloc::format!("UART open: {:?}", e));
            return;
        }
    };

    if uart.init().is_err() {
        fail!("UART init");
        return;
    }

    if uart.enable_loopback().is_err() {
        fail!("UART enable_loopback");
        return;
    }
    pass!("UART LBE loopback enabled");

    if uart.send(0xA5).is_err() {
        fail!("UART send 0xA5");
        let _ = uart.disable_loopback();
        return;
    }

    // QEMU LBE is synchronous — byte appears in RX FIFO immediately; spin limit
    // guards against any QEMU version where there is a one-cycle delay.
    let mut received = None;
    for _ in 0..10_000 {
        if uart.rx_ready() {
            match uart.receive() {
                Ok(b)  => { received = Some(b); break; }
                Err(e) => {
                    fail!(alloc::format!("UART receive error: {:?}", e));
                    let _ = uart.disable_loopback();
                    return;
                }
            }
        }
    }

    let _ = uart.disable_loopback();

    match received {
        Some(0xA5) => pass!("UART loopback 0xA5 → 0xA5 correct"),
        Some(b)    => fail!(alloc::format!("UART loopback: expected 0xA5, got 0x{:02X}", b)),
        None       => fail!("UART loopback: rx_ready never asserted within spin limit"),
    }
}

// ── Scenario D: MMIO cap rejection ───────────────────────────────────────────
//
// request_mmio for a range NOT in the kernel allowlist must return PermissionDenied.
// 0x0800_0000 is absent from all arch ALLOWED entries.
fn scenario_cap_reject() {
    println("[D] MMIO cap rejection (out-of-allowlist address)");

    match request_region(0x0800_0000, 0x1000) {
        Err(ViError::PermissionDenied) => pass!("out-of-allowlist → PermissionDenied"),
        Ok(_)  => fail!("out-of-allowlist unexpectedly granted"),
        Err(e) => fail!(alloc::format!("unexpected error: {:?}", e)),
    }
}
