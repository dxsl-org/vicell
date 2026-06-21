#![no_std]
#![no_main]
// #[no_mangle] on main() requires allowing unsafe_attr — all peripheral access is unsafe-free.

extern crate alloc;

use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use driver_pwm_gpio::BitBangPwm;
use hal_pwm::ViPwm;
use ostd::io::println;
use ostd::syscall::sys_yield;
use types::ViError;

// Gate under gpio manifest flag — bit-bang PWM is GPIO in disguise (same precedent as SPI).
declare_manifest!(block_io = false, network = false, spawn = false, gpio = true, uart = false);

// Channel 6 = pin 6: avoids I2C (pins 0/1) and SPI (pins 2–5) conflicts.
const CHANNEL: u8 = 6;
const PWM_HZ: u32 = 50; // 50 Hz — standard servo PWM

#[no_mangle]
pub fn main() {
    println("[pwm-demo] PWM bit-bang demo (channel=6/pin=6, 50 Hz servo)");

    // Retry on AlreadyExists: another cell (robot-demo) may hold GPIO briefly.
    let gpio = loop {
        match Pl061Gpio::open() {
            Ok(g) => break g,
            Err(ViError::AlreadyExists) => { sys_yield(); }
            Err(ViError::PermissionDenied) => {
                println("[pwm-demo] unavailable (gpio cap not granted — non-aarch64 target)");
                return;
            }
            Err(_) => {
                println("[pwm-demo] unavailable (GPIO open failed)");
                return;
            }
        }
    };
    run_pwm_demo(gpio);
}

fn run_pwm_demo(gpio: Pl061Gpio) {
    let mut pwm = BitBangPwm::new(gpio);

    // BitBangPwm maps channel N to GPIO pin N; channel 6 → pin 6.
    if let Err(_) = pwm.set_frequency(CHANNEL, PWM_HZ) {
        println("[pwm-demo] set_frequency failed");
        return;
    }
    if let Err(_) = pwm.enable(CHANNEL) {
        println("[pwm-demo] enable failed");
        return;
    }

    // Sweep duty cycle from 0% to 100% in steps of 100‰.
    for step in 0..=10u16 {
        let duty = step * 100; // 0, 100, 200, ..., 1000 per mille
        if pwm.set_duty(CHANNEL, duty).is_err() {
            println("[pwm-demo] set_duty failed");
            return;
        }

        // Drive 1 000 ticks per duty step (≈ 5 full periods at 50 Hz with assumed 10k ticks/s).
        for _ in 0..1_000 {
            pwm.tick();
        }

        let msg = alloc::format!("[pwm-demo] duty={}‰ ({}%) ch={}", duty, step * 10, CHANNEL);
        println(&msg);
    }

    let _ = pwm.disable(CHANNEL);
    println("[pwm-demo] sweep complete");
}
