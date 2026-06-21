#![no_std]
#![no_main]
// #[no_mangle] on main() requires allowing unsafe_attr — cannot use forbid(unsafe_code).
// All peripheral-access code is unsafe-free (uses MmioRegion abstraction).

extern crate alloc;

use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use driver_spi_gpio::BitBangSpi;
use hal_spi::ViSpi;
use ostd::io::println;
use types::ViError;

// Gate on gpio manifest flag (Option A: bit-bang SPI is GPIO in disguise).
// This grants PL061 MMIO access at spawn without requiring a dedicated SPI flag.
declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = false);

#[no_mangle]
pub fn main() {
    println("[spi-demo] SPI bit-bang demo (MOSI=2, MISO=3, SCK=4, CS=5)");

    match Pl061Gpio::open() {
        Ok(gpio) => run_spi_demo(gpio),
        Err(ViError::PermissionDenied) => {
            println("[spi-demo] SPI unavailable (gpio cap not granted — non-aarch64 target)");
        }
        Err(_) => {
            println("[spi-demo] SPI unavailable (GPIO open failed)");
        }
    }
}

fn run_spi_demo(gpio: Pl061Gpio) {
    let mut spi = BitBangSpi::new(gpio);

    // ── TX-only write: primary assertion ─────────────────────────────────────
    // write() clocks out bytes via MOSI/SCK/CS without sampling MISO.
    // On QEMU this validates the full GPIO MMIO path.
    match spi.write(&[0xA5, 0x3C, 0x00]) {
        Ok(()) => println("[spi-demo] SPI TX OK (0xA5 0x3C 0x00)"),
        Err(_) => {
            println("[spi-demo] SPI BusError — TX failed");
            return;
        }
    }

    // ── Full-duplex transfer: MISO floats to 0x00 in QEMU ────────────────────
    // transfer() clocks out tx bytes and simultaneously clocks in rx bytes.
    // QEMU PL061 has no MOSI→MISO loopback; rx will be 0x00 — expected.
    let mut rx = [0xFFu8; 2];
    match spi.transfer(&[0xAA, 0x55], &mut rx) {
        Ok(()) => {
            let msg = alloc::format!(
                "[spi-demo] SPI transfer OK: sent 0xAA 0x55, recv 0x{:02X} 0x{:02X} (QEMU MISO=0)",
                rx[0], rx[1]
            );
            println(&msg);
        }
        Err(_) => {
            println("[spi-demo] SPI BusError — transfer failed");
            return;
        }
    }

    println("[spi-demo] SPI demo complete");
}
