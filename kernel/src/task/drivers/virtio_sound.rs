//! VirtIO sound (virtio-snd) output driver.
//!
//! Wraps `virtio_drivers::device::sound::VirtIOSound` and exposes a single
//! blocking `play()` entry used by the `AudioPlay` syscall. The output PCM
//! format is fixed (signed 16-bit LE, 2 channels, 44100 Hz) so cells only ever
//! hand the kernel raw interleaved S16 frames — no per-stream negotiation in the
//! ABI. The output stream is configured + started lazily on the first `play()`.
//!
//! `pcm_xfer` polls the virtqueue to completion (no IRQ wiring needed); it
//! chunks `frames` into period-sized buffers internally, so callers may pass an
//! arbitrarily long buffer in one call.

use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal as VirtIOHal;
use core::ptr::NonNull;
use virtio_drivers::device::sound::{PcmFeatures, PcmFormat, PcmRate, VirtIOSound};
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

/// Period (one queue buffer) and ring size for the output stream. 4 KiB period
/// at S16/2ch/44100 ≈ 23 ms; a 4-period ring smooths host consumption.
const PERIOD_BYTES: u32 = 4096;
const BUFFER_BYTES: u32 = PERIOD_BYTES * 4;

struct SoundState {
    dev: VirtIOSound<VirtIOHal, MmioTransport>,
    stream_id: u32,
    started: bool,
}

// SAFETY: single-hart cooperative kernel; all access is serialized through the
// SOUND spinlock, and the underlying MMIO device is owned exclusively here.
unsafe impl Send for SoundState {}

pub static SOUND: Spinlock<Option<SoundState>> = Spinlock::new(None);

/// Force-release this module's lock during fault teardown.
///
/// # Safety
/// Single-hart; called only from the fault/panic path with interrupts disabled.
pub unsafe fn force_unlock_locks() {
    SOUND.force_unlock();
}

/// Probe the MMIO bus for a virtio-snd device and store it (unconfigured).
pub fn init_driver() {
    use crate::task::drivers::virtio_common::virtio_slots;
    for slot in virtio_slots() {
        let header = unsafe { NonNull::new_unchecked(slot.base as *mut VirtIOHeader) };
        match unsafe { MmioTransport::new(header) } {
            Ok(transport) => {
                if transport.device_type() == DeviceType::Sound {
                    match VirtIOSound::<VirtIOHal, MmioTransport>::new(transport) {
                        Ok(dev) => {
                            log::info!("VirtIO Sound: initialized at {:#x}", slot.base);
                            *SOUND.lock() = Some(SoundState { dev, stream_id: 0, started: false });
                            return;
                        }
                        Err(e) => {
                            log::warn!("VirtIO Sound init failed at {:#x}: {:?}", slot.base, e)
                        }
                    }
                } else {
                    // Not ours — forget the transport so its Drop does not reset a
                    // device another driver owns (same convention as virtio_gpu).
                    core::mem::forget(transport);
                }
            }
            Err(_) => {}
        }
    }
}

/// Play raw PCM frames (S16LE, 2 channels, 44100 Hz). Blocks until all frames
/// have been transferred to the device. Returns bytes played (0 = no device or
/// error). Configures + starts the output stream on first call.
pub fn play(frames: &[u8]) -> usize {
    let mut guard = SOUND.lock();
    let Some(state) = guard.as_mut() else {
        return 0; // no sound device on this machine
    };

    if !state.started {
        let streams = match state.dev.output_streams() {
            Ok(s) if !s.is_empty() => s,
            _ => {
                log::warn!("VirtIO Sound: no output stream available");
                return 0;
            }
        };
        state.stream_id = streams[0];
        if state
            .dev
            .pcm_set_params(
                state.stream_id,
                BUFFER_BYTES,
                PERIOD_BYTES,
                PcmFeatures::empty(),
                2,
                PcmFormat::S16,
                PcmRate::Rate44100,
            )
            .is_err()
        {
            log::warn!("VirtIO Sound: pcm_set_params failed");
            return 0;
        }
        if state.dev.pcm_prepare(state.stream_id).is_err()
            || state.dev.pcm_start(state.stream_id).is_err()
        {
            log::warn!("VirtIO Sound: pcm_prepare/start failed");
            return 0;
        }
        state.started = true;
        log::info!(
            "VirtIO Sound: output stream {} started (S16LE / 2ch / 44100 Hz)",
            state.stream_id
        );
    }

    match state.dev.pcm_xfer(state.stream_id, frames) {
        Ok(()) => frames.len(),
        Err(e) => {
            log::warn!("VirtIO Sound: pcm_xfer failed: {:?}", e);
            0
        }
    }
}
