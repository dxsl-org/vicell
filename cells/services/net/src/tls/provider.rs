//! ViTlsProvider — verifying CryptoProvider for the embedded TLS build.
//!
//! Wraps a `pki::CertVerifier` (single CA anchor) and `ViTlsClock` so that
//! every TLS handshake in the default build verifies the server certificate
//! chain, validity dates, and hostname.
//!
//! # Security invariant (B3)
//! `verifier()` MUST return `Ok` unconditionally.  `connection.rs:455` in
//! embedded-tls silently skips verification when `verifier()` returns `Err`
//! — a single fallible path = silent MITM.  The unit test `verifier_always_ok`
//! guards this property.
//!
//! This module is only compiled when `tls-roots-embedded` is active (i.e. not
//! in `tls-insecure` builds).

#![cfg(feature = "tls-roots-embedded")]

use embedded_tls::{Aes128GcmSha256, CryptoProvider, TlsError, TlsVerifier};
use embedded_tls::pki::CertVerifier;
use embedded_tls::CryptoRngCore;
use crate::tls::clock::ViTlsClock;
use crate::tls::rng::ViRng;
use crate::tls::roots::ca_cert;

/// Cert buffer size for the leaf certificate held in-memory during handshake.
/// 4096 bytes covers typical ECC certificates with SAN extensions.
const CERT_SIZE: usize = 4096;

/// A verifying TLS crypto provider for ViCell's embedded build.
///
/// Constructed per-handshake inside `socket::TlsSocketEntry::handshake()`.
/// Never stored as a global — the verifier holds per-connection transcript state.
pub struct ViTlsProvider {
    rng: ViRng,
    verifier: CertVerifier<'static, Aes128GcmSha256, ViTlsClock, CERT_SIZE>,
}

impl ViTlsProvider {
    /// Build a provider anchored to the build-selected CA, with hostname verification.
    ///
    /// `hostname` must match the server's certificate CN or SAN.  An empty
    /// hostname is rejected by `handshake()` before this is called, so
    /// `set_hostname_verification` here is always called with a non-empty value.
    ///
    /// # Errors
    /// Returns `TlsError::InsufficientSpace` if `hostname` exceeds 64 characters
    /// (the `pki::CertVerifier` limit documented in spike-00-findings.md).
    pub fn new(rng: ViRng, hostname: &str) -> Result<Self, TlsError> {
        let mut verifier = CertVerifier::new(ca_cert());
        verifier.set_hostname_verification(hostname)?;
        Ok(Self { rng, verifier })
    }
}

impl CryptoProvider for ViTlsProvider {
    type CipherSuite = Aes128GcmSha256;
    // [u8; 64] satisfies AsRef<[u8]>; unused in server-auth (no client cert / mTLS).
    type Signature = [u8; 64];

    fn rng(&mut self) -> impl CryptoRngCore {
        &mut self.rng
    }

    /// Return the certificate verifier — MUST be infallible (B3 guard).
    ///
    /// embedded-tls connection.rs:455 silently skips verification when this
    /// returns Err.  Any `Err` path here = silent MITM.  Never add fallible
    /// logic to this method.
    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, TlsError> {
        Ok(&mut self.verifier)
    }

    // signer() and client_cert() keep their default (no mTLS in this build).
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// B3 guard: verifier() must always return Ok, regardless of the hostname
    /// passed to ViTlsProvider::new().  A single Err path = MITM bypass.
    #[test]
    fn verifier_always_ok() {
        let rng = ViRng::new();
        // Use a valid hostname — set_hostname_verification only errors on
        // strings > 64 chars; a normal hostname always succeeds.
        let mut provider = ViTlsProvider::new(rng, "broker.example.com")
            .expect("ViTlsProvider::new must succeed for a valid hostname");

        // The invariant: verifier() returns Ok unconditionally.
        assert!(
            provider.verifier().is_ok(),
            "verifier() returned Err — B3 violation: embedded-tls would skip verification"
        );
    }

    /// Confirm that a freshly constructed provider also satisfies the invariant.
    /// (Redundant with above, but makes the guard explicit for reviewers.)
    #[test]
    fn verifier_ok_on_fresh_provider() {
        let rng = ViRng::new();
        let mut provider =
            ViTlsProvider::new(rng, "test.internal").expect("new must not fail on short hostname");
        let result = provider.verifier();
        assert!(result.is_ok());
    }
}
