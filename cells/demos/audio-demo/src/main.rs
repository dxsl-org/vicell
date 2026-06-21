//! Audio demo cell — proves the VirtIO sound path end-to-end.
//!
//! Generates a 3-tone arpeggio (A4–C#5–E5) as a square wave and plays it through
//! `sys_audio_play`. PCM format is fixed by the ABI: signed 16-bit LE, 2 channels
//! (interleaved L/R), 44100 Hz. Square wave is used so no `sin()` (libm) is
//! needed — purely integer generation.

#![no_std]
#![no_main]
#![allow(static_mut_refs)] // single-task cell — no data race on the tone buffer

extern crate ostd;

use ostd::io::println;
use ostd::syscall::{sys_audio_play, sys_exit};

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Log, AudioPlay];

const RATE: usize = 44100;
/// 0.4 s per tone, stereo S16 → 4 bytes/frame.
const FRAMES: usize = RATE * 2 / 5;
const BUF_LEN: usize = FRAMES * 4;
const AMP: i16 = 7000; // ~21% of full scale — audible but not harsh

static mut TONE: [u8; BUF_LEN] = [0u8; BUF_LEN];

/// Fill TONE with a `freq` Hz square wave (same sample on L and R).
unsafe fn gen_square(freq: usize) {
    let half = (RATE / (freq * 2)).max(1); // samples per half period
    for i in 0..FRAMES {
        let s: i16 = if (i / half) % 2 == 0 { AMP } else { -AMP };
        let b = s.to_le_bytes();
        let off = i * 4;
        TONE[off] = b[0];
        TONE[off + 1] = b[1]; // left
        TONE[off + 2] = b[0];
        TONE[off + 3] = b[1]; // right
    }
}

#[no_mangle]
pub extern "C" fn main() {
    println("[audio-demo] playing A4-C#5-E5 arpeggio on VirtIO sound...");
    for &freq in &[440usize, 554, 659] {
        unsafe { gen_square(freq) };
        let played = sys_audio_play(unsafe { &TONE });
        if played == 0 {
            println("[audio-demo] no sound device or play failed");
            sys_exit(1);
        }
    }
    println("[audio-demo] done — audio path OK");
    sys_exit(0);
}
