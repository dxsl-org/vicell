//! TlsSocketEntry — persists a TlsConnection across SEND/RECV IPC calls.
//!
//! Uses Box::leak for 16 KiB read/write buffers so TlsConnection<'static> can be
//! stored in the socket table.  Memory cost: 32 KiB per concurrent TLS connection.
//! Max 4 simultaneous TLS connections in G1 robot = 128 KiB overhead total.
//!
//! Build flavors:
//!   default (tls-roots-embedded)  — ViTlsProvider; server certs verified
//!   tls-insecure                  — UnsecureProvider; no verification (dev only)

extern crate alloc;

use alloc::boxed::Box;
use embedded_tls::blocking::{TlsConfig, TlsConnection, TlsContext};
use embedded_tls::{Aes128GcmSha256, TlsError};
use smoltcp::iface::{Interface, SocketHandle};
use crate::interface::VirtioNetDevice;
use crate::tls::rng::ViRng;
use crate::tls::transport::{set_tls_context, SmoltcpTlsTransport};

// Verifying path — default build.
#[cfg(feature = "tls-roots-embedded")]
use crate::tls::provider::ViTlsProvider;

// Insecure path — dev/lab builds only.
#[cfg(feature = "tls-insecure")]
use embedded_tls::UnsecureProvider;

// A TLS flavor that actually drives `conn.open()` MUST be selected. Without this
// guard, a build with neither feature (e.g. the unimplemented `tls-roots-full`, or
// `--no-default-features`) would compile, skip the handshake entirely, and return a
// TLS entry with ZERO verification — a silent bypass. (haily-reviewer MAJOR-1.)
#[cfg(not(any(feature = "tls-roots-embedded", feature = "tls-insecure")))]
compile_error!(
    "service-net: select a TLS flavor — `tls-roots-embedded` (verifying) or `tls-insecure` (dev only). \
     `tls-roots-full` is not yet implemented (see plan phase-04)."
);

/// TLS record buffer size (covers the max 16 KB TLS record + overhead).
const TLS_BUF: usize = 16640;

/// A live TLS connection stored in the socket table.
///
/// After construction the TLS handshake has already completed.  Before calling
/// `write()` or `read()` the caller must call `set_tls_context()` so the embedded
/// transport can drive smoltcp.
pub struct TlsSocketEntry {
    pub conn: TlsConnection<'static, SmoltcpTlsTransport, Aes128GcmSha256>,
    pub handle: SocketHandle,
}

impl TlsSocketEntry {
    /// Perform the TLS 1.3 handshake over `handle` and return a live entry.
    ///
    /// Blocks (spin-polls smoltcp via the global transport context) until the
    /// handshake completes or times out.
    ///
    /// In verifying builds (default), an empty `hostname` is rejected before
    /// `open()` to produce a clear error rather than a silent SAN mismatch.
    ///
    /// # Safety
    /// `set_tls_context()` must have been called with valid pointers before this.
    pub unsafe fn handshake(
        handle:   SocketHandle,
        hostname: &str,
    ) -> Result<Self, TlsError> {
        // Leak 32 KiB for TLS record buffers — intentional, documented cost.
        let read_buf:  &'static mut [u8] = Box::leak(Box::new([0u8; TLS_BUF]));
        let write_buf: &'static mut [u8] = Box::leak(Box::new([0u8; TLS_BUF]));

        let transport = SmoltcpTlsTransport::new(handle);
        let mut conn  = TlsConnection::new(transport, read_buf, write_buf);

        let config = if hostname.is_empty() {
            TlsConfig::new()
        } else {
            TlsConfig::new().with_server_name(hostname)
        };

        // ── Provider selection ────────────────────────────────────────────────
        //
        // tls-roots-embedded (default): verifying path.
        //   Empty hostname rejected here for a clear diagnostic before open().
        //   ViTlsProvider::new() sets hostname verification on CertVerifier.
        //
        // tls-insecure: accept-all path, dev/lab only.
        //   Banner printed once on first handshake.

        #[cfg(feature = "tls-roots-embedded")]
        {
            if hostname.is_empty() {
                return Err(TlsError::InvalidCertificate);
            }
            let rng = ViRng::new();
            let provider = ViTlsProvider::new(rng, hostname)?;
            let ctx = TlsContext::new(&config, provider);
            conn.open(ctx)?;
        }

        #[cfg(feature = "tls-insecure")]
        {
            // One-time banner: printed on every handshake in insecure builds so
            // the danger is never invisible in logs.
            // Safety: single-core net cell; concurrent call not possible.
            static BANNER_PRINTED: core::sync::atomic::AtomicBool =
                core::sync::atomic::AtomicBool::new(false);
            if !BANNER_PRINTED.swap(true, core::sync::atomic::Ordering::Relaxed) {
                // The message is intentionally alarming — it must not be missed.
                // sys_log is best-effort; we don't propagate errors.
                let _ = ostd::syscall::sys_log(
                    "[net/tls] !!! INSECURE TLS BUILD - server certs NOT verified !!!"
                );
            }

            let rng = ViRng::new();
            let ctx = TlsContext::new(&config, UnsecureProvider::new::<Aes128GcmSha256>(rng));
            conn.open(ctx)?;
        }

        Ok(Self { conn, handle })
    }

    /// Convenience: set TLS context and write data.
    ///
    /// # Safety
    /// Same as `set_tls_context`.
    pub unsafe fn send(
        &mut self,
        data:    &[u8],
        iface:   *mut Interface,
        device:  *mut VirtioNetDevice,
        sockets: *mut (),
    ) -> Result<usize, TlsError> {
        set_tls_context(iface, device, sockets);
        self.conn.write(data)
    }

    /// Convenience: set TLS context and read data.
    ///
    /// # Safety
    /// Same as `set_tls_context`.
    pub unsafe fn recv(
        &mut self,
        buf:     &mut [u8],
        iface:   *mut Interface,
        device:  *mut VirtioNetDevice,
        sockets: *mut (),
    ) -> Result<usize, TlsError> {
        set_tls_context(iface, device, sockets);
        self.conn.read(buf)
    }
}
