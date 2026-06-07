//! SmoltcpTlsTransport — smoltcp TCP socket as an embedded-io Read+Write transport.
//!
//! Since `TlsConnection` does not expose `delegate_mut()`, the smoltcp context
//! pointers (iface/device/sockets) are stored in module-level AtomicPtrs.
//! Before any TLS I/O call, invoke `set_tls_context()` with the current mutable
//! references.  Safe because the net cell is single-core.

extern crate alloc;

use core::sync::atomic::{AtomicPtr, Ordering};
use embedded_io::{ErrorKind, ErrorType, Read, Write};
use smoltcp::{
    iface::{Interface, SocketHandle, SocketSet},
    socket::tcp,
    time::Instant,
};
use crate::interface::VirtioNetDevice;
use ostd::syscall::sys_get_time;

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
        const MAX_SPIN: u32 = 5_000_000;
        let mut spins = 0u32;
        loop {
            // SAFETY: set_tls_context was called before open()/read().
            unsafe { Self::poll(); }
            let n = unsafe {
                let socket = Self::sockets().get_mut::<tcp::Socket>(self.handle);
                if socket.can_recv() { socket.recv_slice(buf).unwrap_or(0) } else { 0 }
            };
            if n > 0 { return Ok(n); }
            spins += 1;
            if spins > MAX_SPIN {
                return Err(ErrorKind::TimedOut);
            }
            core::hint::spin_loop();
        }
    }
}

impl Write for SmoltcpTlsTransport {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ErrorKind> {
        const MAX_SPIN: u32 = 1_000_000;
        let mut written = 0usize;
        let mut spins = 0u32;
        while written < buf.len() {
            let n = unsafe {
                let socket = Self::sockets().get_mut::<tcp::Socket>(self.handle);
                if socket.can_send() { socket.send_slice(&buf[written..]).unwrap_or(0) } else { 0 }
            };
            if n > 0 {
                written += n;
                spins = 0;
            } else {
                unsafe { Self::poll(); }
                spins += 1;
                if spins > MAX_SPIN {
                    return Err(ErrorKind::TimedOut);
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
