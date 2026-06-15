//! Raw L2 frame IPC bridge: hypervisor cell ↔ Net Cell.
//!
//! `transmit`: guest TX frame → Net Cell L2Send → kernel NIC TX.
//! `try_receive`: Net Cell L2Recv → one inbound frame for the guest RX queue, or None.
//!
//! Both functions block until the Net Cell acknowledges (it always replies synchronously).

extern crate alloc;
use alloc::boxed::Box;
use api::ipc::{self, IPC_BUF_SIZE, NetRequest, NetResponse};
use ostd::syscall::{sys_send, sys_recv};

use crate::virtio_net::GUEST_MAC;

/// Forward a raw Ethernet frame to the Net Cell for NIC TX.
///
/// Blocks until the Net Cell sends its `Ok` acknowledgement.  No-op when `net_tid == 0`.
pub fn transmit(net_tid: usize, frame: &[u8]) {
    if net_tid == 0 { return; }
    let req = NetRequest::L2Send { data: frame };
    let mut buf = [0u8; IPC_BUF_SIZE];
    let Ok(msg) = ipc::encode(&req, &mut buf) else { return; };
    sys_send(net_tid, msg);
    // Drain the mandatory Ok response to prevent mailbox accumulation.
    let mut rb = [0u8; IPC_BUF_SIZE];
    sys_recv(0, &mut rb);
}

/// Poll the Net Cell for one inbound Ethernet frame destined for the guest MAC.
///
/// Returns `Some(frame)` if a frame was available, `None` otherwise.
/// Blocks for one round-trip to the Net Cell.  No-op when `net_tid == 0`.
pub fn try_receive(net_tid: usize) -> Option<Box<[u8]>> {
    if net_tid == 0 { return None; }
    let req = NetRequest::L2Recv { guest_mac: GUEST_MAC };
    let mut buf = [0u8; IPC_BUF_SIZE];
    let Ok(msg) = ipc::encode(&req, &mut buf) else { return None; };
    sys_send(net_tid, msg);
    let mut rb = [0u8; IPC_BUF_SIZE];
    sys_recv(0, &mut rb);
    match ipc::decode::<NetResponse<'_>>(&rb) {
        Ok(NetResponse::Data(frame)) if !frame.is_empty() => Some(Box::from(frame)),
        _ => None,
    }
}
