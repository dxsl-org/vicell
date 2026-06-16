#![no_std]
#![no_main]

extern crate alloc;

use api::declare_manifest;
use driver_can_loopback::LoopbackCan;
use hal_can::{CanFrame, ViCan};
use ostd::io::println;

// No MMIO required — loopback only. No capability flags needed.
declare_manifest!(block_io = false, network = false, spawn = false, gpio = false, uart = false);

const FRAME_COUNT: u32 = 5;
const BASE_ID: u32 = 0x100;
const KBPS: u32 = 500;

#[no_mangle]
pub fn main() {
    println("[can-demo] CAN loopback demo (500 kbps, 5 frames)");

    let mut can = LoopbackCan::new();

    if can.configure(KBPS).is_err() {
        println("[can-demo] configure failed");
        return;
    }

    // Transmit 5 standard frames.
    for seq in 0..FRAME_COUNT {
        let id = BASE_ID + seq;
        let payload = [id as u8, seq as u8, 0xAA, 0x55];
        let frame = CanFrame::new(id, &payload);

        if can.send_frame(&frame).is_err() {
            println("[can-demo] TX failed");
            return;
        }

        let msg = alloc::format!(
            "[can-demo] TX id=0x{:03X} dlc={} data=[{:#04X},{:#04X},{:#04X},{:#04X}]",
            frame.id, frame.dlc, frame.data[0], frame.data[1], frame.data[2], frame.data[3]
        );
        println(&msg);
    }

    // Receive all 5 frames back (loopback: same buffer).
    for expected_seq in 0..FRAME_COUNT {
        let expected_id = BASE_ID + expected_seq;

        match can.recv_frame() {
            Ok(frame) => {
                if frame.id != expected_id {
                    let msg = alloc::format!(
                        "[can-demo] RX id mismatch: got 0x{:03X}, expected 0x{:03X}",
                        frame.id, expected_id
                    );
                    println(&msg);
                    return;
                }
                let msg = alloc::format!(
                    "[can-demo] RX id=0x{:03X} dlc={} OK",
                    frame.id, frame.dlc
                );
                println(&msg);
            }
            Err(_) => {
                println("[can-demo] RX failed (unexpected RxEmpty)");
                return;
            }
        }
    }

    println("[can-demo] loopback OK — all 5 frames round-tripped");
}
