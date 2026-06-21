// SPDX-License-Identifier: MPL-2.0
// BSD socket shims: socket / connect / send / recv / close
//
// Socket fd range: 10–17 (above stdio 0–2 and shell-reserved 3–9).
// The fd→cap_id mapping is stored in SOCK_CAPS[] (one slot per fd).
//
// Thread safety: ViCell is single-hart for G1; AtomicU32 CAS prevents
// double-alloc from interrupt context.

#![allow(unsafe_code)]

use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use crate::ipc::{decode, encode, IPC_BUF_SIZE, NetRequest, NetResponse};
use crate::syscall::ViSyscall;
use super::sysio::raw_syscall;

pub(super) const SOCK_BASE_FD: c_int = 10;
const MAX_SOCKETS: usize = 8;

const AF_INET: c_int = 2;
const SOCK_STREAM: c_int = 1;

static NET_TID_CACHE: AtomicUsize = AtomicUsize::new(0);

/// cap_id slot per socket fd. 0 = free, u32::MAX = reserved (alloc in progress).
static SOCK_CAPS: [AtomicU32; MAX_SOCKETS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];

/// Internet socket address (mirrors `struct sockaddr_in`).
#[repr(C)]
pub struct sockaddr_in {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: u32,
    pub sin_zero: [u8; 8],
}

fn net_tid() -> usize {
    let cached = NET_TID_CACHE.load(Ordering::Relaxed);
    if cached != 0 { return cached; }
    // LookupService = 206, service::NET = 2
    let tid = unsafe { raw_syscall(ViSyscall::LookupService, 2, 0, 0, 0) };
    if tid > 0 {
        NET_TID_CACHE.store(tid as usize, Ordering::Relaxed);
        tid as usize
    } else {
        0
    }
}

fn alloc_fd() -> Option<(c_int, usize)> {
    for i in 0..MAX_SOCKETS {
        if SOCK_CAPS[i].compare_exchange(0, u32::MAX, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return Some((SOCK_BASE_FD + i as c_int, i));
        }
    }
    None
}

fn cap_from_fd(fd: c_int) -> Option<u32> {
    let idx = fd - SOCK_BASE_FD;
    if idx < 0 || idx as usize >= MAX_SOCKETS { return None; }
    let cap = SOCK_CAPS[idx as usize].load(Ordering::Acquire);
    if cap == 0 || cap == u32::MAX { None } else { Some(cap) }
}

#[no_mangle]
pub unsafe extern "C" fn socket(domain: c_int, type_: c_int, _protocol: c_int) -> c_int {
    if domain != AF_INET || type_ != SOCK_STREAM { return -1; }
    match alloc_fd() { Some((fd, _)) => fd, None => -1 }
}

#[no_mangle]
pub unsafe extern "C" fn connect(fd: c_int, addr: *const c_void, addrlen: c_int) -> c_int {
    let idx = fd - SOCK_BASE_FD;
    if idx < 0 || idx as usize >= MAX_SOCKETS { return -1; }
    if addr.is_null() || addrlen < core::mem::size_of::<sockaddr_in>() as c_int { return -1; }
    let net = net_tid();
    if net == 0 { return -1; }

    let sin = addr as *const sockaddr_in;
    if (*sin).sin_family != AF_INET as u16 { return -1; }
    let ip = (*sin).sin_addr.to_be_bytes();
    let port = u16::from_be((*sin).sin_port);

    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = NetRequest::TcpConnect { addr: ip, port };
    let Ok(encoded) = encode(&req, &mut req_buf) else { return -1; };
    raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);

    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let n = raw_syscall(ViSyscall::Recv, 0, resp_buf.as_mut_ptr() as usize, resp_buf.len(), 0);
    if n <= 0 { return -1; }

    match decode::<NetResponse>(&resp_buf[..n as usize]) {
        Ok(NetResponse::CapId(cap)) if cap > 0 => {
            SOCK_CAPS[idx as usize].store(cap, Ordering::Release);
            0
        }
        _ => -1,
    }
}

/// Send up to 495 bytes per call (IPC payload ceiling after postcard framing).
#[no_mangle]
pub unsafe extern "C" fn send(fd: c_int, buf: *const c_void, len: usize, _flags: c_int) -> c_int {
    if buf.is_null() { return -1; }
    let Some(cap) = cap_from_fd(fd) else { return -1; };
    let net = net_tid();
    if net == 0 { return -1; }

    let capped = len.min(495);
    let data = core::slice::from_raw_parts(buf as *const u8, capped);
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = NetRequest::TcpSend { cap_id: cap, data };
    let Ok(encoded) = encode(&req, &mut req_buf) else { return -1; };

    for _attempt in 0..20 {
        raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        let n = raw_syscall(ViSyscall::Recv, 0, resp_buf.as_mut_ptr() as usize, resp_buf.len(), 0);
        if n <= 0 { return -1; }
        match decode::<NetResponse>(&resp_buf[..n as usize]) {
            Ok(NetResponse::Data(bytes)) if bytes.len() >= 4 => {
                let accepted = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if accepted > 0 || capped == 0 {
                    return (accepted as i32).min(capped as i32);
                }
                raw_syscall(ViSyscall::Yield, 0, 0, 0, 0);
            }
            _ => return -1,
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn recv(fd: c_int, buf: *mut c_void, len: usize, _flags: c_int) -> c_int {
    if buf.is_null() { return -1; }
    let Some(cap) = cap_from_fd(fd) else { return -1; };
    let net = net_tid();
    if net == 0 { return -1; }

    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = NetRequest::TcpRecv { cap_id: cap, buf_len: len as u32 };
    let Ok(encoded) = encode(&req, &mut req_buf) else { return -1; };
    raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);

    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let n = raw_syscall(ViSyscall::Recv, 0, resp_buf.as_mut_ptr() as usize, resp_buf.len(), 0);
    if n <= 0 { return 0; }

    match decode::<NetResponse>(&resp_buf[..n as usize]) {
        Ok(NetResponse::Data(data)) => {
            let copy_len = data.len().min(len);
            core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, copy_len);
            copy_len as c_int
        }
        _ => -1,
    }
}

// _close dispatches socket fds here; regular fds go to the kernel Close syscall.
#[no_mangle]
pub unsafe extern "C" fn _close(handle: c_int) -> c_int {
    if handle >= SOCK_BASE_FD && handle < SOCK_BASE_FD + MAX_SOCKETS as c_int {
        return socket_close(handle);
    }
    raw_syscall(ViSyscall::Close, handle as usize, 0, 0, 0) as c_int
}

unsafe fn socket_close(fd: c_int) -> c_int {
    let idx = (fd - SOCK_BASE_FD) as usize;
    let cap = SOCK_CAPS[idx].load(Ordering::Acquire);
    if cap == 0 { return -1; }

    if cap != u32::MAX {
        let net = net_tid();
        if net != 0 {
            let mut req_buf = [0u8; IPC_BUF_SIZE];
            let req = NetRequest::TcpClose { cap_id: cap };
            if let Ok(encoded) = encode(&req, &mut req_buf) {
                raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);
                let mut r = [0u8; 4];
                raw_syscall(ViSyscall::Recv, 0, r.as_mut_ptr() as usize, r.len(), 0);
            }
        }
    }
    SOCK_CAPS[idx].store(0, Ordering::Release);
    0
}
