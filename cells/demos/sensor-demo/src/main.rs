#![no_std]
#![no_main]

extern crate alloc;

mod sht3x;

use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use driver_i2c_gpio::BitBangI2c;
use hal_i2c::ViI2c;
use ostd::io::println;
use ostd::syscall::sys_recv_timeout;

// Declare gpio capability so the kernel grants PL061 MMIO access at spawn.
declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = false);

#[no_mangle]
pub fn main() {
    println("[sensor-demo] SHT3x via bit-bang I2C (pin 0=SCL, pin 1=SDA, addr 0x44)");

    match Pl061Gpio::open() {
        Ok(gpio) => run_with_gpio(gpio),
        Err(_) => {
            println("[sensor-demo] GPIO unavailable — synthetic-only mode");
            run_synthetic();
        }
    }
}

// Bounded so GPIO is released for other Driver Cells (e.g. pwm-demo, spi-demo).
const DEMO_CYCLES: u32 = 3;

fn run_with_gpio(gpio: Pl061Gpio) {
    let mut i2c = BitBangI2c::new(gpio);
    for tick in 0..DEMO_CYCLES {
        let r = poll_sensor(&mut i2c, tick);
        print_reading(&r);
        sleep_1s();
    }
}

fn poll_sensor(i2c: &mut impl ViI2c<Error = hal_i2c::I2cError>, tick: u32) -> sht3x::Reading {
    // SHT3x high-precision single-shot: write [0x2C, 0x06], read 6 bytes.
    let mut buf = [0u8; 6];
    match i2c.write_read(0x44, &[0x2C, 0x06], &mut buf) {
        Ok(()) => sht3x::parse(&buf).unwrap_or_else(|| sht3x::synthetic(tick)),
        // NackAddress: no slave on bus — expected in QEMU without a real sensor.
        Err(_) => sht3x::synthetic(tick),
    }
}

fn print_reading(r: &sht3x::Reading) {
    let label  = if r.simulated { " [sim]" } else { "" };
    let t_int  = r.temp_cx10 / 10;
    let t_frac = (r.temp_cx10 % 10).abs();
    // When temp is between -0.9 and -0.1°C, t_int == 0 but the value is still negative.
    let t_sign = if r.temp_cx10 < 0 && t_int == 0 { "-" } else { "" };
    let h_int  = r.hum_px10 / 10;
    let h_frac = r.hum_px10 % 10;
    println(alloc::format!("T={}{}.{}C H={}.{}%{}", t_sign, t_int, t_frac, h_int, h_frac, label).as_str());
}

fn run_synthetic() {
    for tick in 0..DEMO_CYCLES {
        print_reading(&sht3x::synthetic(tick));
        sleep_1s();
    }
}

/// Block for approximately 1 s.
///
/// Uses `RecvTimeout` as a sleep primitive: 100 scheduler ticks × 10 ms/tick.
/// A stray message wakes us early — fine for a polling demo, we just loop again.
fn sleep_1s() {
    let mut buf = [0u8; 64];
    let _ = sys_recv_timeout(0, &mut buf, 100);
}
