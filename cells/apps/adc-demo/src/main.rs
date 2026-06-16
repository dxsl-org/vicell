#![no_std]
#![no_main]

extern crate alloc;

use api::declare_manifest;
use driver_adc_sim::{SimAdc, SimChannel};
use hal_adc::ViAdc;
use ostd::io::println;

// No MMIO required — simulation only. No capability flags needed.
declare_manifest!(block_io = false, network = false, spawn = false, gpio = false, uart = false);

const SUPPLY_MV: u32 = 3300; // 3.3 V reference

#[no_mangle]
pub fn main() {
    println("[adc-demo] ADC simulation demo (3 channels, 12-bit, 3.3V ref)");

    let mut adc = SimAdc::new();

    // Channel 0: slow temperature-like ramp (5 s cycle), with noise.
    adc.configure(0, SimChannel::new(5_000, 3));
    // Channel 1: medium ramp (2 s cycle), clean.
    adc.configure(1, SimChannel::new(2_000, 0));
    // Channel 2: fast ramp (800 step cycle), with noise.
    adc.configure(2, SimChannel::new(800, 2));

    for i in 0..5u32 {
        adc.step();

        let r0 = adc.read_raw(0).unwrap_or(0);
        let r1 = adc.read_raw(1).unwrap_or(0);
        let r2 = adc.read_raw(2).unwrap_or(0);

        let mv0 = adc.to_millivolts(r0, SUPPLY_MV);
        let mv1 = adc.to_millivolts(r1, SUPPLY_MV);
        let mv2 = adc.to_millivolts(r2, SUPPLY_MV);

        let msg = alloc::format!(
            "[adc-demo] step={} ch0={}mV ch1={}mV ch2={}mV (raw {}/{}/{})",
            i, mv0, mv1, mv2, r0, r1, r2
        );
        println(&msg);
    }

    println("[adc-demo] done");
}
