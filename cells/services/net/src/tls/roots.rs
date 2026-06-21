//! Single trust anchor for the embedded TLS verifier.
//!
//! `ca_cert()` returns the DER-encoded CA certificate selected at build time
//! by the active `tls-ca-*` cargo feature.  Exactly one CA selector must be
//! active; the compiler rejects zero or multiple via `compile_error!`.
//!
//! The returned `Certificate` borrows `'static` bytes embedded in the binary
//! — no heap allocation.
//!
//! Adding a new CA:
//!   1. Place `roots/<name>.der` (DER, not PEM).
//!   2. Add a cargo feature `tls-ca-<name>` in Cargo.toml.
//!   3. Add an `#[cfg]` arm below.
//!   4. Document it in roots/README.md.

#[cfg(any(
    feature = "tls-ca-private",
    feature = "tls-ca-amazon",
    feature = "tls-ca-letsencrypt",
    feature = "tls-ca-rsa",
))]
use embedded_tls::Certificate;

// ── Feature guard: exactly one CA selector must be active ─────────────────────

#[cfg(all(feature = "tls-ca-private", feature = "tls-ca-amazon"))]
compile_error!(
    "tls-ca-private and tls-ca-amazon are mutually exclusive. Enable exactly one CA selector."
);
#[cfg(all(feature = "tls-ca-private", feature = "tls-ca-letsencrypt"))]
compile_error!(
    "tls-ca-private and tls-ca-letsencrypt are mutually exclusive. Enable exactly one CA selector."
);
#[cfg(all(feature = "tls-ca-private", feature = "tls-ca-rsa"))]
compile_error!(
    "tls-ca-private and tls-ca-rsa are mutually exclusive. Enable exactly one CA selector."
);
#[cfg(all(feature = "tls-ca-amazon", feature = "tls-ca-letsencrypt"))]
compile_error!(
    "tls-ca-amazon and tls-ca-letsencrypt are mutually exclusive. Enable exactly one CA selector."
);
#[cfg(all(feature = "tls-ca-amazon", feature = "tls-ca-rsa"))]
compile_error!(
    "tls-ca-amazon and tls-ca-rsa are mutually exclusive. Enable exactly one CA selector."
);
#[cfg(all(feature = "tls-ca-letsencrypt", feature = "tls-ca-rsa"))]
compile_error!(
    "tls-ca-letsencrypt and tls-ca-rsa are mutually exclusive. Enable exactly one CA selector."
);

// In verifying builds (tls-roots-embedded active), at least one CA must be selected.
#[cfg(all(
    feature = "tls-roots-embedded",
    not(feature = "tls-insecure"),
    not(feature = "tls-ca-private"),
    not(feature = "tls-ca-amazon"),
    not(feature = "tls-ca-letsencrypt"),
    not(feature = "tls-ca-rsa"),
))]
compile_error!(
    "tls-roots-embedded requires exactly one CA selector feature \
     (tls-ca-private, tls-ca-amazon, tls-ca-letsencrypt, or tls-ca-rsa)."
);

// ── DER bytes ─────────────────────────────────────────────────────────────────

/// ECDSA P-256 self-signed CA.  Replace `roots/private.der` with your fleet CA.
/// Source: generated at image build; see roots/README.md.
/// notAfter: 2036-06-18.
#[cfg(feature = "tls-ca-private")]
static CA_DER: &[u8] = include_bytes!("../../roots/private.der");

/// Amazon Root CA 3 — ECDSA P-256.
/// Source: https://www.amazontrust.com/repository/AmazonRootCA3.pem
/// notAfter: 2040-05-26.
#[cfg(feature = "tls-ca-amazon")]
static CA_DER: &[u8] = include_bytes!("../../roots/amazon-root-ca3.der");

/// ISRG Root X2 — ECDSA P-384.
/// Source: https://letsencrypt.org/certs/isrg-root-x2.pem
/// notAfter: 2040-09-17.
#[cfg(feature = "tls-ca-letsencrypt")]
static CA_DER: &[u8] = include_bytes!("../../roots/isrg-x2.der");

/// RSA CA (heavyweight opt-in, +135 KB).  Replace with your RSA CA DER.
/// No default DER is shipped — you must supply `roots/rsa-ca.der`.
#[cfg(feature = "tls-ca-rsa")]
static CA_DER: &[u8] = include_bytes!("../../roots/rsa-ca.der");

// ── Public API ────────────────────────────────────────────────────────────────

/// Return the active trust anchor as a `Certificate` borrowing `'static` bytes.
///
/// The CA is selected at compile time by the active `tls-ca-*` cargo feature.
/// In `tls-insecure` builds this function is present but never called by the
/// handshake path (which uses `UnsecureProvider` instead).
#[cfg(any(
    feature = "tls-ca-private",
    feature = "tls-ca-amazon",
    feature = "tls-ca-letsencrypt",
    feature = "tls-ca-rsa",
))]
pub fn ca_cert() -> Certificate<&'static [u8]> {
    Certificate::X509(CA_DER)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(
        feature = "tls-ca-private",
        feature = "tls-ca-amazon",
        feature = "tls-ca-letsencrypt",
        feature = "tls-ca-rsa",
    ))]
    #[test]
    fn ca_der_is_nonempty() {
        // Sanity check: the embedded DER must be non-empty and at least
        // long enough to contain an ASN.1 SEQUENCE header (minimum 2 bytes).
        // A zero-length or truncated DER would cause a decode error at handshake.
        assert!(CA_DER.len() >= 2, "CA DER is suspiciously short");
        // DER certificates start with SEQUENCE tag (0x30).
        assert_eq!(CA_DER[0], 0x30, "CA DER does not start with SEQUENCE tag");
    }

    #[cfg(any(
        feature = "tls-ca-private",
        feature = "tls-ca-amazon",
        feature = "tls-ca-letsencrypt",
        feature = "tls-ca-rsa",
    ))]
    #[test]
    fn ca_cert_returns_x509() {
        let cert = ca_cert();
        assert!(
            matches!(cert, Certificate::X509(_)),
            "ca_cert() must return Certificate::X509"
        );
    }
}
