// SPDX-License-Identifier: MPL-2.0
// P-256 cryptographic operations for the silo guest.
//
// Key design rules:
// - The private key never leaves this module.
// - `init` zeros the seed buffer before returning (fail or succeed).
// - All results are stack-allocated; no heap required.
// - ECDSA uses RFC 6979 deterministic nonces (no RNG needed after init).
// - ECDH uses the static signing key for a static-static exchange.

use p256::{
    ecdsa::{DerSignature, SigningKey, VerifyingKey},
    ecdh::diffie_hellman,
    elliptic_curve::sec1::FromEncodedPoint,
    EncodedPoint, PublicKey,
};
// PrehashSigner::sign_prehash — signs a pre-computed digest without hashing.
use p256::ecdsa::signature::hazmat::PrehashSigner;

/// Opaque state held inside the silo event loop.
pub struct SiloState {
    // `None` until `Init` completes successfully.
    key: Option<SigningKey>,
}

/// Result of a single crypto operation.
///
/// Stack-only: largest variant is 72 bytes for a DER signature.
pub enum CryptoResult {
    /// `Init` succeeded.  `pub_key` = uncompressed SEC1 point (65 bytes).
    Ready { pub_key: [u8; 65] },
    /// `Sign` succeeded.  `der[..len]` = DER-encoded signature (≤ 72 bytes).
    Signature { der: [u8; 72], len: u8 },
    /// `Ecdh` succeeded.  Inner value = 32-byte raw shared secret.
    SharedSecret([u8; 32]),
    /// `GetPub` succeeded.  Inner value = uncompressed SEC1 public key (65 bytes).
    PubKey([u8; 65]),
    /// Operation failed.  Inner value = error code byte.
    Fault(u8),
}

impl SiloState {
    /// Create an uninitialised state.  `Init` must be called before any other op.
    pub const fn uninit() -> Self {
        Self { key: None }
    }

    /// Initialise the silo key from a 32-byte entropy seed.
    ///
    /// The seed buffer is zeroed before this function returns, regardless of
    /// whether the key derivation succeeded.
    ///
    /// Error codes:
    /// - `0x01` — seed is not a valid P-256 scalar (all-zero or ≥ order).
    pub fn init(&mut self, seed: &mut [u8; 32]) -> CryptoResult {
        // Attempt key derivation first, then zero regardless of outcome.
        let result = SigningKey::from_bytes(seed.as_slice().into());
        // Zero the seed — key material must not persist on stack after return.
        seed.iter_mut().for_each(|b| *b = 0);

        match result {
            Ok(key) => {
                let pub_key = uncompressed_pub(&key);
                self.key = Some(key);
                CryptoResult::Ready { pub_key }
            }
            Err(_) => CryptoResult::Fault(0x01),
        }
    }

    /// Sign a pre-hashed 32-byte digest with the silo key.
    ///
    /// Error codes:
    /// - `0x10` — key not initialised (call `Init` first).
    /// - `0x11` — signing failed (should not occur with valid key + RFC 6979).
    /// - `0x12` — DER encoding longer than 72 bytes (should not occur for P-256).
    pub fn sign(&self, digest: &[u8; 32]) -> CryptoResult {
        let Some(ref key) = self.key else {
            return CryptoResult::Fault(0x10);
        };

        // sign_prehash: RFC 6979 deterministic k; no RNG required.
        let sig: p256::ecdsa::Signature = match key.sign_prehash(digest) {
            Ok(s) => s,
            Err(_) => return CryptoResult::Fault(0x11),
        };

        // Encode as ASN.1 DER.
        let der_sig = DerSignature::from(sig);
        let der_bytes = der_sig.as_bytes();

        if der_bytes.len() > 72 {
            return CryptoResult::Fault(0x12);
        }

        let mut der = [0u8; 72];
        let len = der_bytes.len() as u8;
        der[..der_bytes.len()].copy_from_slice(der_bytes);
        CryptoResult::Signature { der, len }
    }

    /// Perform static-static ECDH with the given uncompressed peer public key.
    ///
    /// Error codes:
    /// - `0x20` — key not initialised.
    /// - `0x21` — peer bytes are not a valid SEC1 encoded point.
    /// - `0x22` — peer encoded point is not on the P-256 curve.
    pub fn ecdh(&self, peer_pub_bytes: &[u8; 65]) -> CryptoResult {
        let Some(ref key) = self.key else {
            return CryptoResult::Fault(0x20);
        };

        let peer_point = match EncodedPoint::from_bytes(peer_pub_bytes.as_slice()) {
            Ok(p) => p,
            Err(_) => return CryptoResult::Fault(0x21),
        };

        // from_encoded_point returns subtle::CtOption<PublicKey>; use into_option().
        let peer_key = match PublicKey::from_encoded_point(&peer_point).into_option() {
            Some(k) => k,
            None => return CryptoResult::Fault(0x22),
        };

        // diffie_hellman accepts impl Borrow<NonZeroScalar<C>>.
        // SigningKey::as_nonzero_scalar() returns &NonZeroScalar<NistP256>,
        // which satisfies Borrow<NonZeroScalar<NistP256>>.
        let shared = diffie_hellman(key.as_nonzero_scalar(), peer_key.as_affine());

        let mut out = [0u8; 32];
        out.copy_from_slice(shared.raw_secret_bytes().as_ref());
        CryptoResult::SharedSecret(out)
    }

    /// Return the silo's own uncompressed public key.
    ///
    /// Error code:
    /// - `0x30` — key not initialised.
    pub fn get_pub(&self) -> CryptoResult {
        let Some(ref key) = self.key else {
            return CryptoResult::Fault(0x30);
        };
        CryptoResult::PubKey(uncompressed_pub(key))
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the uncompressed SEC1 encoding of a signing key's public key.
///
/// Returns exactly 65 bytes: `04 || X || Y`.
fn uncompressed_pub(key: &SigningKey) -> [u8; 65] {
    let vk = VerifyingKey::from(key);
    // to_encoded_point(false) = uncompressed; always 65 bytes for P-256.
    let point = vk.to_encoded_point(false);
    let bytes = point.as_bytes();
    debug_assert_eq!(bytes.len(), 65, "P-256 uncompressed point must be 65 bytes");
    let mut out = [0u8; 65];
    out.copy_from_slice(bytes);
    out
}
