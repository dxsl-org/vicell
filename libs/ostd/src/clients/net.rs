// SPDX-License-Identifier: MPL-2.0

//! Network service client — ergonomic TCP/UDP access.

extern crate alloc;

use alloc::vec::Vec;
use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use crate::{ViError, ViResult};
use crate::service::NetRef;
use super::vierr_from_code;

/// Max bytes sent per [`TcpStream::write`] call.
///
/// Conservative margin below `IPC_BUF_SIZE` to leave room for postcard framing
/// (`TcpSend` discriminant + cap_id varint + data length varint ≈ 10 bytes).
const MAX_TCP_WRITE: usize = IPC_BUF_SIZE - 256;

/// A TCP/UDP socket handle returned by [`NetClient::tcp_connect`].
pub type SocketId = u32;

/// Ergonomic client for the network service.
///
/// Wraps [`NetRef`] and hides request construction + postcard encoding.
pub struct NetClient {
    svc: NetRef,
}

impl NetClient {
    /// Create a new unresolved client. Resolution is lazy (first call).
    pub fn new() -> Self {
        Self { svc: NetRef::new() }
    }

    /// Open a TCP connection to `addr:port`.
    ///
    /// Returns a `SocketId` on success.  The socket is owned by the net service;
    /// call [`tcp_close`][Self::tcp_close] when done.
    pub fn tcp_connect(&mut self, addr: [u8; 4], port: u16) -> ViResult<SocketId> {
        let req = NetRequest::TcpConnect { addr, port };
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        match self.svc.call::<NetRequest, NetResponse>(&req, &mut resp_buf)? {
            NetResponse::CapId(id) => Ok(id),
            NetResponse::Err(code) => Err(vierr_from_code(code)),
            _ => Err(ViError::IO),
        }
    }

    /// Send `data` over an established TCP socket.
    ///
    /// The net service replies `Data(n)` where `n` (4-byte LE) is the number of
    /// bytes the socket accepted — `0` while the connection is still completing
    /// its handshake (SynSent). Returns `WouldBlock` if not all bytes were
    /// accepted so the caller can retry (after a yield) once the socket is
    /// Established. `data` should be ≤ the socket's tx buffer (4 KiB) per call.
    pub fn tcp_send(&mut self, id: SocketId, data: &[u8]) -> ViResult<()> {
        let req = NetRequest::TcpSend { cap_id: id, data };
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        match self.svc.call::<NetRequest, NetResponse>(&req, &mut resp_buf)? {
            NetResponse::Data(b) => {
                let n = if b.len() >= 4 {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize
                } else {
                    0
                };
                if n == data.len() {
                    Ok(())
                } else {
                    Err(ViError::WouldBlock)
                }
            }
            NetResponse::Ok => Ok(()),
            NetResponse::Err(code) => Err(vierr_from_code(code)),
            _ => Err(ViError::IO),
        }
    }

    /// Receive up to `buf_len` bytes from an established TCP socket.
    pub fn tcp_recv(&mut self, id: SocketId, buf_len: u32) -> ViResult<Vec<u8>> {
        let req = NetRequest::TcpRecv { cap_id: id, buf_len };
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        match self.svc.call::<NetRequest, NetResponse>(&req, &mut resp_buf)? {
            NetResponse::Data(data) => Ok(data.to_vec()),
            NetResponse::Ok => Ok(alloc::vec![]),
            NetResponse::Err(code) => Err(vierr_from_code(code)),
            _ => Err(ViError::IO),
        }
    }

    /// Close a TCP connection.
    pub fn tcp_close(&mut self, id: SocketId) -> ViResult<()> {
        let req = NetRequest::TcpClose { cap_id: id };
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        match self.svc.call::<NetRequest, NetResponse>(&req, &mut resp_buf)? {
            NetResponse::Ok => Ok(()),
            NetResponse::Err(code) => Err(vierr_from_code(code)),
            _ => Err(ViError::IO),
        }
    }

    /// Resolve a hostname to an IPv4 address.
    pub fn dns_lookup(&mut self, hostname: &str) -> ViResult<[u8; 4]> {
        let req = NetRequest::Resolve { hostname };
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        match self.svc.call::<NetRequest, NetResponse>(&req, &mut resp_buf)? {
            NetResponse::Addr(addr) => Ok(addr),
            NetResponse::Err(code) => Err(vierr_from_code(code)),
            _ => Err(ViError::IO),
        }
    }

    /// Return the DHCP-assigned local IPv4 address.
    pub fn local_ip(&mut self) -> ViResult<[u8; 4]> {
        let req = NetRequest::GetLocalIp;
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        match self.svc.call::<NetRequest, NetResponse>(&req, &mut resp_buf)? {
            NetResponse::Addr(addr) => Ok(addr),
            NetResponse::Err(code) => Err(vierr_from_code(code)),
            _ => Err(ViError::IO),
        }
    }
}

impl Default for NetClient {
    fn default() -> Self { Self::new() }
}

// ─── TcpStream ────────────────────────────────────────────────────────────────

/// A TCP connection handle implementing [`embedded_io::Read`] and [`embedded_io::Write`].
///
/// Obtained via [`TcpStream::connect`]. The underlying socket is closed when
/// `TcpStream` is dropped (RAII — Law 8).
///
/// # Blocking contract
/// [`Read::read`] blocks (with cooperative yields) until at least one byte is
/// available, matching the [`embedded_io`] blocking contract.  The net service
/// returns data non-blockingly per call; the loop in `read` handles the wait.
///
/// # Write chunking
/// [`Write::write`] sends at most [`MAX_TCP_WRITE`] bytes per call due to the
/// IPC buffer limit.  Use [`write_all`][embedded_io::WriteExt::write_all] when
/// all bytes must be delivered.
pub struct TcpStream {
    client: NetClient,
    id: SocketId,
}

impl TcpStream {
    /// Open a TCP connection to `addr:port`.
    ///
    /// Resolves the net service lazily on the first call.
    pub fn connect(addr: [u8; 4], port: u16) -> ViResult<Self> {
        let mut client = NetClient::new();
        let id = client.tcp_connect(addr, port)?;
        Ok(Self { client, id })
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        let _ = self.client.tcp_close(self.id);
    }
}

impl embedded_io::ErrorType for TcpStream {
    type Error = crate::io::OstdError;
}

impl embedded_io::Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, crate::io::OstdError> {
        loop {
            let recv_len = buf.len().min(u32::MAX as usize) as u32;
            match self.client.tcp_recv(self.id, recv_len) {
                Ok(data) if !data.is_empty() => {
                    let n = data.len().min(buf.len());
                    buf[..n].copy_from_slice(&data[..n]);
                    return Ok(n);
                }
                Ok(_) => {
                    // No data yet — yield and retry.
                    crate::task::yield_now();
                }
                Err(ViError::Unknown) => {
                    // EOF: net service sent Err(0xFF) signalling FIN/RST.
                    return Ok(0);
                }
                Err(e) => return Err(crate::io::OstdError(e)),
            }
        }
    }
}

impl embedded_io::Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize, crate::io::OstdError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let chunk = buf.len().min(MAX_TCP_WRITE);
        self.client
            .tcp_send(self.id, &buf[..chunk])
            .map_err(crate::io::OstdError)?;
        Ok(chunk)
    }

    fn flush(&mut self) -> Result<(), crate::io::OstdError> {
        Ok(()) // tcp_send commits synchronously through the net service
    }
}
