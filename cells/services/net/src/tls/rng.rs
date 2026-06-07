//! ViRng — VirtIO-RNG-backed ChaCha20 PRNG for embedded-tls.
//!
//! Seeds a ChaCha20Rng from sys_get_random (hardware entropy) once on construction.
//! Loops until 32 bytes of entropy are collected (device may return < 32 per call).
//! Panics if no RNG device is present — the net cell must never attempt TLS without
//! real entropy (mtime-seeded PRNG was deliberately removed as a fallback).

use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use ostd::syscall::sys_get_random;

/// ChaCha20 PRNG seeded from VirtIO-RNG hardware entropy.
pub struct ViRng(ChaCha20Rng);

impl ViRng {
    /// Seed from VirtIO-RNG. Loops until all 32 seed bytes are filled.
    ///
    /// # Panics
    /// Panics if VirtIO-RNG is absent (returns 0 bytes consistently after 64 attempts).
    pub fn new() -> Self {
        let mut seed = [0u8; 32];
        let mut filled = 0usize;
        let mut attempts = 0u32;
        while filled < 32 {
            let n = sys_get_random(&mut seed[filled..]);
            if n > 0 {
                filled += n;
            } else {
                attempts += 1;
                // After 64 failed attempts, the device is not present.
                assert!(
                    attempts < 64,
                    "[net/tls] VirtIO-RNG absent — cannot generate TLS entropy. \
                     Add -object rng-random,id=rng0 -device virtio-rng-device,rng=rng0 to QEMU."
                );
                for _ in 0..1000 { core::hint::spin_loop(); }
            }
        }
        Self(ChaCha20Rng::from_seed(seed))
    }
}

impl RngCore for ViRng {
    fn next_u32(&mut self) -> u32 { self.0.next_u32() }
    fn next_u64(&mut self) -> u64 { self.0.next_u64() }
    fn fill_bytes(&mut self, dest: &mut [u8]) { self.0.fill_bytes(dest) }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.0.try_fill_bytes(dest)
    }
}

impl CryptoRng for ViRng {}
