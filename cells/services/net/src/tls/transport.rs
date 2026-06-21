//! SmoltcpTlsTransport — smoltcp TCP socket as an embedded-io Read+Write transport.
//!
//! Since `TlsConnection` does not expose `delegate_mut()`, the smoltcp context
//! pointers (iface/device/sockets) are stored in module-level AtomicPtrs.
//! Before any TLS I/O call, invoke `set_tls_context()` with the current mutable
//! references.  Safe because the net cell is single-core.
//!
//! # Transport timeouts (M4)
//! Both `Read` and `Write` spin-poll smoltcp.  The original iteration-count cap
//! (`MAX_SPIN`) was replaced with a wall-clock deadline: a software ECDSA/RSA
//! verify on QEMU TCG can take tens of milliseconds and runs *between* `read()`
//! calls, easily exceeding a spin count while the work is still making progress.
//! Using `sys_get_time()` (10 MHz monotonic counter) ties the timeout to real
//! elapsed time regardless of CPU speed.
//!
//! `sys_heartbeat()` is called inside both spin loops so a long but progressing
//! verify does not trip the RT watchdog.  The wall-clock deadline still fires
//! if the transport genuinely hangs.

extern crate alloc;

use core::sync::atomic::{AtomicPtr, Ordering};
use embedded_io::{ErrorKind, ErrorType, Read, Write};
use smoltcp::{
    iface::{Interface, SocketHandle, SocketSet},
    socket::tcp,
    time::Instant,
};
use crate::interface::VirtioNetDevice;
use ostd::syscall::{sys_get_time, sys_heartbeat};

// ── Thread-local smoltcp context (single-core net cell) ───────────────────────

static TLS_IFACE:   AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static TLS_DEVICE:  AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static TLS_SOCKETS: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Update module-level smoltcp context pointers before any TLS I/O call.
///
/// # Safety
/// Caller must hold valid `&mut` references to all three objects. On single-core
/// hardware the absence of data races makes this safe for the duration of one IPC
/// handler invocation.
pub unsafe fn set_tls_context(
    iface:   *mut Interface,
    device:  *mut VirtioNetDevice,
    sockets: *mut (),
) {
    TLS_IFACE  .store(iface   as *mut (), Ordering::Relaxed);
    TLS_DEVICE .store(device  as *mut (), Ordering::Relaxed);
    TLS_SOCKETS.store(sockets,            Ordering::Relaxed);
}

// ── Timeout constants ─────────────────────────────────────────────────────────

/// Wall-clock TLS I/O timeout in sys_get_time ticks (10 MHz monotonic clock).
///
/// 30 seconds: generous enough for software RSA-PSS verify on QEMU TCG (~1-5 s
/// measured), with headroom for a loaded system.  P03 e2e will tighten if needed.
/// Formula: 30 s * 10_000_000 ticks/s = 300_000_000.
const TLS_IO_TIMEOUT_TICKS: u64 = 300_000_000;

/// Heartbeat interval: reset the RT watchdog every 500 ms worth of ticks while
/// spinning.  500 ms * 10_000_000 ticks/s = 5_000_000 ticks.
const HEARTBEAT_INTERVAL_TICKS: u64 = 5_000_000;

// ── Transport implementation ──────────────────────────────────────────────────

/// Wraps a smoltcp TCP socket handle as an embedded-io Read+Write transport.
///
/// Reads the smoltcp context from the module-level AtomicPtrs set by `set_tls_context`.
pub struct SmoltcpTlsTransport {
    pub handle: SocketHandle,
}

// SAFETY: Net cell is single-core; AtomicPtrs are updated under exclusive access.
unsafe impl Send for SmoltcpTlsTransport {}
unsafe impl Sync for SmoltcpTlsTransport {}

impl SmoltcpTlsTransport {
    pub fn new(handle: SocketHandle) -> Self {
        Self { handle }
    }

    unsafe fn iface()   -> &'static mut Interface {
        &mut *(TLS_IFACE.load(Ordering::Relaxed) as *mut Interface)
    }
    unsafe fn device()  -> &'static mut VirtioNetDevice {
        &mut *(TLS_DEVICE.load(Ordering::Relaxed) as *mut VirtioNetDevice)
    }
    unsafe fn sockets() -> &'static mut SocketSet<'static> {
        &mut *(TLS_SOCKETS.load(Ordering::Relaxed) as *mut SocketSet<'static>)
    }

    fn now() -> Instant {
        Instant::from_micros((sys_get_time() / 10) as i64)
    }

    unsafe fn poll() {
        Self::device().pump_rx();
        Self::iface().poll(Self::now(), Self::device(), Self::sockets());
    }
}

impl ErrorType for SmoltcpTlsTransport {
    type Error = ErrorKind;  // embedded_io::ErrorKind implements embedded_io::Error
}

impl Read for SmoltcpTlsTransport {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ErrorKind> {
        let deadline = sys_get_time() + TLS_IO_TIMEOUT_TICKS;
        let mut next_heartbeat = sys_get_time() + HEARTBEAT_INTERVAL_TICKS;

        loop {
            // SAFETY: set_tls_context was called before open()/read().
            unsafe { Self::poll(); }
            let n = unsafe {
                let socket = Self::sockets().get_mut::<tcp::Socket>(self.handle);
                if socket.can_recv() { socket.recv_slice(buf).unwrap_or(0) } else { 0 }
            };
            if n > 0 { return Ok(n); }

            let now = sys_get_time();
            if now >= deadline {
                return Err(ErrorKind::TimedOut);
            }
            // Periodically reset the RT watchdog so a slow software verify
            // (ECDSA/RSA on QEMU TCG) does not look like a hung task.
            // The deadline above still fires if the transport genuinely hangs.
            if now >= next_heartbeat {
                sys_heartbeat(500);
                next_heartbeat = now + HEARTBEAT_INTERVAL_TICKS;
            }
            core::hint::spin_loop();
        }
    }
}

impl Write for SmoltcpTlsTransport {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ErrorKind> {
        let deadline = sys_get_time() + TLS_IO_TIMEOUT_TICKS;
        let mut next_heartbeat = sys_get_time() + HEARTBEAT_INTERVAL_TICKS;
        let mut written = 0usize;

        while written < buf.len() {
            let n = unsafe {
                let socket = Self::sockets().get_mut::<tcp::Socket>(self.handle);
                if socket.can_send() { socket.send_slice(&buf[written..]).unwrap_or(0) } else { 0 }
            };
            if n > 0 {
                written += n;
                // Note: `deadline` is a fixed total-write budget (30s), not reset on
                // progress. Handshake writes are small, so this is ample; revisit only
                // if bulk TLS writes ever need a progress-reset deadline.
            } else {
                unsafe { Self::poll(); }

                let now = sys_get_time();
                if now >= deadline {
                    return Err(ErrorKind::TimedOut);
                }
                if now >= next_heartbeat {
                    sys_heartbeat(500);
                    next_heartbeat = now + HEARTBEAT_INTERVAL_TICKS;
                }
                core::hint::spin_loop();
            }
        }
        unsafe { Self::poll(); }
        Ok(written)
    }

    fn flush(&mut self) -> Result<(), ErrorKind> {
        unsafe { Self::poll(); }
        Ok(())
    }
}
