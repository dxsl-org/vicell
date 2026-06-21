#![no_std]
#![no_main]

//! Reference robot demo — G1 graduation criterion 8.
//!
//! Pipeline: SHT3x I2C sensor read → threshold compute → GPIO relay actuator → MQTT telemetry.
//! Falls back to synthetic data when no I2C slave responds (expected on QEMU).
//! Falls back to pure simulation when GPIO is unavailable (RISC-V target).

extern crate alloc;

mod mqtt;
mod sht3x;

use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use driver_i2c_gpio::BitBangI2c;
use hal_gpio::{PinDir, ViGpio};
use hal_i2c::ViI2c;
use ostd::io::println;
use types::ViError;

declare_manifest!(block_io = false, network = true, spawn = false, gpio = true, uart = false);
api::declare_syscalls![Send, Recv, RecvTimeout, Log, LookupService, Heartbeat, RequestMmio];

const ACTUATOR_PIN:        u8  = 3;
const LOOP_CYCLES:         u32 = 5;
/// 25.0 °C threshold — relay ON when temperature exceeds this.
const TEMP_THRESHOLD_CX10: i32 = 250;

#[no_mangle]
pub fn main() {
    println("[robot-demo] ViCell reference robot demo (G1 graduation criterion 8)");

    let gpio = match Pl061Gpio::open() {
        Ok(g) => g,
        Err(ViError::PermissionDenied) => {
            println("[robot-demo] GPIO not available — running simulation");
            simulate_loop();
            return;
        }
        Err(e) => {
            let msg = alloc::format!("[robot-demo] GPIO open error: {:?}", e);
            println(&msg);
            return;
        }
    };

    run_with_gpio(gpio);
}

/// Real hardware path (AArch64 / any target with PL061 GPIO).
///
/// Each iteration cycles GPIO ownership through `BitBangI2c` for the sensor
/// read (pins 0=SCL, 1=SDA), then reclaims it via `into_gpio()` for the
/// actuator write (pin 3). Pin 3 direction is re-asserted every iteration
/// because the I2C path may leave it as Input.
fn run_with_gpio(mut gpio: Pl061Gpio) {
    println("[robot-demo] GPIO open — SHT3x I2C sensor loop (5 cycles)");
    for tick in 0..LOOP_CYCLES {
        let mut i2c = BitBangI2c::new(gpio);
        let reading = poll_sensor(&mut i2c, tick);
        gpio = i2c.into_gpio();

        let relay = reading.temp_cx10 > TEMP_THRESHOLD_CX10;
        let _ = gpio.set_direction(ACTUATOR_PIN, PinDir::Output);
        let _ = gpio.write_pin(ACTUATOR_PIN, relay);
        print_reading(&reading, relay);
        publish_cycle(&reading, relay);
        ostd::task::yield_now();
    }
    let _ = gpio.write_pin(ACTUATOR_PIN, false);
    println("[robot-demo] done (5 cycles)");
}

/// Simulation path (RISC-V or any target without GPIO MMIO access).
fn simulate_loop() {
    for tick in 0..LOOP_CYCLES {
        let reading = sht3x::synthetic(tick);
        let relay = reading.temp_cx10 > TEMP_THRESHOLD_CX10;
        print_reading(&reading, relay);
        publish_cycle(&reading, relay);
        ostd::task::yield_now();
    }
    println("[robot-demo] done (5 cycles)");
}

fn poll_sensor(i2c: &mut impl ViI2c<Error = hal_i2c::I2cError>, tick: u32) -> sht3x::Reading {
    let mut buf = [0u8; 6];
    match i2c.write_read(0x44, &[0x2C, 0x06], &mut buf) {
        Ok(()) => sht3x::parse(&buf).unwrap_or_else(|| sht3x::synthetic(tick)),
        Err(_) => sht3x::synthetic(tick),
    }
}

fn print_reading(r: &sht3x::Reading, relay: bool) {
    let label  = if r.simulated { " [sim]" } else { "" };
    let t_int  = r.temp_cx10 / 10;
    let t_frac = (r.temp_cx10 % 10).abs();
    let t_sign = if r.temp_cx10 < 0 && t_int == 0 { "-" } else { "" };
    let h_int  = r.hum_px10 / 10;
    let h_frac = r.hum_px10 % 10;
    println(&alloc::format!(
        "[robot-demo] T={}{}.{}C H={}.{}%{} relay={}",
        t_sign, t_int, t_frac, h_int, h_frac, label,
        if relay { "on" } else { "off" }
    ));
}

fn publish_cycle(r: &sht3x::Reading, relay: bool) {
    let payload = alloc::format!(
        r#"{{"t_cx10":{},"h_px10":{},"relay":{}}}"#,
        r.temp_cx10, r.hum_px10, relay
    );
    mqtt::publish_telemetry(&payload);
}
