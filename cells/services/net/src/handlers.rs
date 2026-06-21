//! Typed NetRequest dispatch and TLS raw fallback for the net service cell.
//!
//! Entry point is `handle_request`: tries postcard decode first; on failure
//! routes opcode ≥ 0x30 to the legacy TLS raw path (used by `ostd::tls` helpers).

extern crate alloc;

use alloc::collections::BTreeMap;
use api::ipc::{self, IPC_BUF_SIZE, NetRequest, NetResponse};
use ostd::syscall::{sys_send, sys_heartbeat, sys_net_tx};
use smoltcp::{
    iface::{Interface, SocketHandle, SocketSet},
    socket::{tcp, udp},
    wire::{IpAddress, IpEndpoint},
};
use crate::{
    interface::VirtioNetDevice,
    next_ephemeral_port, now_instant,
    socket_state::SocketState,
    socket_table::SocketTable,
    tls::socket::TlsSocketEntry,
};

// ─── TLS raw opcode constants (ostd::tls wire format) ────────────────────────

const TLS_CLOSE_OP:   u8 = 0x15;  // mirrors ostd::tls CLOSE; lowest raw opcode
const TLS_CONNECT_OP: u8 = 0x30;
const TLS_SEND_OP:    u8 = 0x31;
const TLS_RECV_OP:    u8 = 0x32;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn tcp_state_byte(s: tcp::State) -> u8 {
    match s {
        tcp::State::Closed      => 0x00,
        tcp::State::SynSent     => 0x01,
        tcp::State::SynReceived => 0x02,
        tcp::State::Established => 0x03,
        tcp::State::FinWait1    => 0x04,
        tcp::State::FinWait2    => 0x05,
        tcp::State::CloseWait   => 0x06,
        tcp::State::Closing     => 0x07,
        tcp::State::LastAck     => 0x08,
        tcp::State::TimeWait    => 0x09,
        tcp::State::Listen      => 0x0A,
    }
}

fn send_typed(sender: usize, resp: NetResponse<'_>) {
    let mut r = [0u8; IPC_BUF_SIZE];
    if let Ok(s) = ipc::encode(&resp, &mut r) {
        sys_send(sender, s);
    }
}

/// Promote a Connecting socket to Connected once smoltcp reaches Established.
fn try_promote(table: &mut SocketTable, sockets: &mut SocketSet<'_>, cap: u64) {
    if table.get_state(cap) == Some(SocketState::Connecting) {
        if let Some(h) = table.get(cap) {
            if sockets.get_mut::<tcp::Socket>(h).state() == tcp::State::Established {
                table.set_state(cap, SocketState::Connected);
            }
        }
    }
}

fn make_tcp(
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
) -> Result<(SocketHandle, u64), ()> {
    let handle = sockets.add(tcp::Socket::new(
        tcp::SocketBuffer::new(alloc::vec![0u8; 4096]),
        tcp::SocketBuffer::new(alloc::vec![0u8; 4096]),
    ));
    match table.insert(handle) {
        Ok(cap) => Ok((handle, cap)),
        Err(_) => { sockets.remove(handle); Err(()) }
    }
}

// ─── Top-level router ─────────────────────────────────────────────────────────

/// Decode and dispatch one IPC message from the net service main loop.
///
/// Tries postcard-decode first.  On failure, routes opcodes ≥ 0x30 to the
/// legacy TLS raw path (`ostd::tls` helpers).  Everything else is silently
/// dropped (malformed / stale data).
#[allow(clippy::too_many_arguments)]
pub fn handle_request(
    buf: &[u8],
    sender: usize,
    iface: &mut Interface,
    device: &mut VirtioNetDevice,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    tls_table: &mut BTreeMap<u64, TlsSocketEntry>,
    local_ip: &[u8; 4],
) {
    match ipc::decode::<NetRequest<'_>>(buf) {
        Ok(req) => {
            iface.poll(now_instant(), device, sockets);
            handle_typed(req, sender, iface, device, sockets, table, tls_table, local_ip);
            iface.poll(now_instant(), device, sockets);
        }
        Err(_) if buf.first().copied().unwrap_or(0) >= TLS_CLOSE_OP => {
            iface.poll(now_instant(), device, sockets);
            handle_tls_raw(buf, sender, iface, device, sockets, table, tls_table);
            iface.poll(now_instant(), device, sockets);
        }
        _ => {}
    }
}

// ─── Typed dispatch ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn handle_typed(
    req: NetRequest<'_>,
    sender: usize,
    iface: &mut Interface,
    device: &mut VirtioNetDevice,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    tls_table: &mut BTreeMap<u64, TlsSocketEntry>,
    local_ip: &[u8; 4],
) {
    use NetResponse as R;
    match req {
        NetRequest::TcpConnect { addr, port } => {
            let (handle, cap) = match make_tcp(sockets, table) {
                Ok(t) => t,
                Err(_) => { send_typed(sender, R::Err(0xFF)); return; }
            };
            let remote = IpEndpoint::new(IpAddress::v4(addr[0], addr[1], addr[2], addr[3]), port);
            {
                let cx = iface.context();
                if sockets.get_mut::<tcp::Socket>(handle)
                    .connect(cx, remote, next_ephemeral_port()).is_err()
                {
                    table.remove(cap); sockets.remove(handle);
                    send_typed(sender, R::Err(0xFF)); return;
                }
            }
            table.set_state(cap, SocketState::Connecting);
            send_typed(sender, R::CapId(cap as u32));
        }

        NetRequest::TcpSend { cap_id, data } => {
            let cap = cap_id as u64;
            if table.is_udp(cap) { send_typed(sender, R::Data(&0u32.to_le_bytes())); return; }
            try_promote(table, sockets, cap);
            let n = if let Some(h) = table.get(cap) {
                let s = sockets.get_mut::<tcp::Socket>(h);
                if s.can_send() { s.send_slice(data).unwrap_or(0) } else { 0 }
            } else { 0 };
            send_typed(sender, R::Data(&(n as u32).to_le_bytes()));
        }

        NetRequest::TcpRecv { cap_id, buf_len } => {
            let cap = cap_id as u64;
            if table.is_udp(cap) { send_typed(sender, R::Data(&[])); return; }
            try_promote(table, sockets, cap);
            let buf_len = (buf_len as usize).min(4096);
            let mut data = alloc::vec![0u8; buf_len];
            if let Some(h) = table.get(cap) {
                let s = sockets.get_mut::<tcp::Socket>(h);
                if s.can_recv() {
                    let n = s.recv_slice(&mut data).unwrap_or(0);
                    send_typed(sender, R::Data(&data[..n]));
                } else if !s.may_recv() {
                    // Receive half closed (FIN/RST received) — signal EOF to caller.
                    send_typed(sender, R::Err(0xFF));
                } else {
                    send_typed(sender, R::Data(&[]));
                }
            } else {
                send_typed(sender, R::Data(&[]));
            }
        }

        NetRequest::TcpClose { cap_id } => {
            let cap = cap_id as u64;
            if let Some(h) = table.remove(cap) { sockets.remove(h); }
            tls_table.remove(&cap);
            send_typed(sender, R::Ok);
        }

        NetRequest::TcpListen { port } => {
            let (handle, cap) = match make_tcp(sockets, table) {
                Ok(t) => t,
                Err(_) => { send_typed(sender, R::Err(0xFF)); return; }
            };
            if sockets.get_mut::<tcp::Socket>(handle).listen(port).is_err() {
                table.remove(cap); sockets.remove(handle);
                send_typed(sender, R::Err(0xFF)); return;
            }
            table.set_state(cap, SocketState::Listening);
            table.set_listen_port(cap, port);
            send_typed(sender, R::CapId(cap as u32));
        }

        NetRequest::TcpAccept { cap_id } => {
            let cap = cap_id as u64;
            if table.is_udp(cap) || table.get_state(cap) != Some(SocketState::Listening) {
                send_typed(sender, R::Err(0xFF)); return;
            }
            let handle = match table.get(cap) {
                Some(h) => h,
                None => { send_typed(sender, R::Err(0xFF)); return; }
            };
            if sockets.get_mut::<tcp::Socket>(handle).state() != tcp::State::Established {
                send_typed(sender, R::Err(0xFE)); return;  // not ready — consumer retries
            }
            let listen_port = match table.get_listen_port(cap) {
                Some(p) => p,
                None => { send_typed(sender, R::Err(0xFF)); return; }
            };
            match table.insert_with_state(handle, SocketState::Connected) {
                Ok(stream_cap) => {
                    let mut ns = tcp::Socket::new(
                        tcp::SocketBuffer::new(alloc::vec![0u8; 4096]),
                        tcp::SocketBuffer::new(alloc::vec![0u8; 4096]),
                    );
                    let _ = ns.listen(listen_port);
                    let nh = sockets.add(ns);
                    table.update_handle(cap, nh);
                    table.set_state(cap, SocketState::Listening);
                    table.set_listen_port(cap, listen_port);
                    send_typed(sender, R::CapId(stream_cap as u32));
                }
                Err(_) => { send_typed(sender, R::Err(0xFF)); }
            }
        }

        NetRequest::UdpCreate => {
            let handle = sockets.add(udp::Socket::new(
                udp::PacketBuffer::new(alloc::vec![udp::PacketMetadata::EMPTY; 4], alloc::vec![0u8; 1024]),
                udp::PacketBuffer::new(alloc::vec![udp::PacketMetadata::EMPTY; 4], alloc::vec![0u8; 1024]),
            ));
            match table.insert(handle) {
                Ok(cap) => { table.mark_udp(cap); send_typed(sender, R::CapId(cap as u32)); }
                Err(_)  => { sockets.remove(handle); send_typed(sender, R::Err(0xFF)); }
            }
        }

        NetRequest::UdpBind { cap_id, port } => {
            let cap = cap_id as u64;
            let port = if port == 0 { next_ephemeral_port() } else { port };
            let ok = if let Some(h) = table.get(cap) {
                sockets.get_mut::<udp::Socket>(h).bind(port).is_ok()
            } else { false };
            if ok { table.set_state(cap, SocketState::Listening); send_typed(sender, R::Ok); }
            else  { send_typed(sender, R::Err(0xFF)); }
        }

        NetRequest::UdpSend { cap_id, addr, port, data } => {
            let cap = cap_id as u64;
            let ep = IpEndpoint::new(IpAddress::v4(addr[0], addr[1], addr[2], addr[3]), port);
            let n = if let Some(h) = table.get(cap) {
                let result = sockets.get_mut::<udp::Socket>(h).send_slice(data, ep);
                if result.is_ok() {
                    iface.poll(now_instant(), device, sockets);
                    data.len()
                } else { 0 }
            } else { 0 };
            send_typed(sender, R::Data(&(n as u32).to_le_bytes()));
        }

        NetRequest::UdpRecv { cap_id, buf_len } => {
            let cap = cap_id as u64;
            let buf_len = (buf_len as usize).min(512);
            let result = if let Some(h) = table.get(cap) {
                let s = sockets.get_mut::<udp::Socket>(h);
                if s.can_recv() {
                    let mut raw = alloc::vec![0u8; buf_len];
                    s.recv_slice(&mut raw).ok().map(|(n, meta)| {
                        let IpAddress::Ipv4(src_ip) = meta.endpoint.addr;
                        let mut reply = alloc::vec![0u8; 6 + n];
                        reply[0..4].copy_from_slice(src_ip.as_bytes());
                        reply[4..6].copy_from_slice(&meta.endpoint.port.to_le_bytes());
                        reply[6..6+n].copy_from_slice(&raw[..n]);
                        reply
                    })
                } else { None }
            } else { None };
            match result {
                Some(reply) => {
                    let mut rb = [0u8; IPC_BUF_SIZE];
                    if let Ok(s) = ipc::encode(&R::Data(&reply), &mut rb) { sys_send(sender, s); }
                }
                None => { send_typed(sender, R::Data(&[])); }
            }
        }

        NetRequest::SocketState { cap_id } => {
            let cap = cap_id as u64;
            if table.is_udp(cap) { send_typed(sender, R::State(0x00)); return; }
            let byte = if let Some(h) = table.get(cap) {
                tcp_state_byte(sockets.get_mut::<tcp::Socket>(h).state())
            } else { 0x00 };
            send_typed(sender, R::State(byte));
        }

        NetRequest::GetLocalIp => {
            send_typed(sender, R::Addr(*local_ip));
        }

        NetRequest::MulticastJoin { cap_id: _, group } => {
            let g = IpAddress::v4(group[0], group[1], group[2], group[3]);
            let ok = iface.join_multicast_group(device, g, now_instant()).is_ok();
            send_typed(sender, if ok { R::Ok } else { R::Err(0xFF) });
        }

        NetRequest::MulticastLeave { cap_id: _, group } => {
            let g = IpAddress::v4(group[0], group[1], group[2], group[3]);
            let ok = iface.leave_multicast_group(device, g, now_instant()).is_ok();
            send_typed(sender, if ok { R::Ok } else { R::Err(0xFF) });
        }

        NetRequest::Resolve { .. } => {
            send_typed(sender, R::Err(0xFF));  // DNS resolver not yet implemented
        }

        NetRequest::L2Send { data } => {
            // Bypass smoltcp: forward raw Ethernet frame directly to the kernel NIC TX.
            sys_net_tx(data);
            send_typed(sender, R::Ok);
        }

        NetRequest::L2Recv { guest_mac } => {
            // Register guest MAC (idempotent) then pump the NIC for fresh frames.
            device.set_guest_mac(guest_mac);
            device.pump_rx_split();
            if let Some(frame) = device.pop_guest_rx() {
                let mut rb = [0u8; IPC_BUF_SIZE];
                if let Ok(s) = ipc::encode(&R::Data(&frame), &mut rb) {
                    sys_send(sender, s);
                }
            } else {
                send_typed(sender, R::Ok);
            }
        }
    }
}

// ─── TLS raw fallback ─────────────────────────────────────────────────────────
//
// `ostd::tls` helpers use the legacy wire format: [opcode:1][cap:8 LE][payload:*].
// postcard decode fails on these (discriminants 0x30–0x32 are out of range).
// A zero-trim extracts the actual payload because the IPC buffer is pre-zeroed.

#[allow(clippy::too_many_arguments)]
fn handle_tls_raw(
    buf: &[u8],
    sender: usize,
    iface: &mut Interface,
    device: &mut VirtioNetDevice,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    tls_table: &mut BTreeMap<u64, TlsSocketEntry>,
) {
    if buf.len() < 9 { sys_send(sender, &[0u8; 8]); return; }
    let opcode  = buf[0];
    let cap     = u64::from_le_bytes(buf[1..9].try_into().unwrap_or([0; 8]));
    // Trim trailing zeros — raw wire has no length prefix; buf is pre-zeroed before recv.
    let actual  = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(9).max(9);
    let payload = &buf[9..actual];

    match opcode {
        TLS_CLOSE_OP => {
            // Reply mirrors old cell_opcodes::CLOSE: 1 zero byte.
            if let Some(h) = table.remove(cap) { sockets.remove(h); }
            tls_table.remove(&cap);
            sys_send(sender, &[0x00]);
        }

        TLS_CONNECT_OP => {
            // payload: [addr:4][port:2 LE][hostname:*]
            if payload.len() < 6 { sys_send(sender, &[0u8; 8]); return; }
            let addr     = [payload[0], payload[1], payload[2], payload[3]];
            let port     = u16::from_le_bytes([payload[4], payload[5]]);
            let hostname = core::str::from_utf8(&payload[6..])
                .unwrap_or("").trim_end_matches('\0');

            let (handle, cap_id) = match make_tcp(sockets, table) {
                Ok(t) => t,
                Err(_) => { sys_send(sender, &[0u8; 8]); return; }
            };
            let remote = IpEndpoint::new(IpAddress::v4(addr[0], addr[1], addr[2], addr[3]), port);
            {
                let cx = iface.context();
                if sockets.get_mut::<tcp::Socket>(handle)
                    .connect(cx, remote, next_ephemeral_port()).is_err()
                {
                    table.remove(cap_id); sockets.remove(handle);
                    sys_send(sender, &[0u8; 8]); return;
                }
            }
            table.set_state(cap_id, SocketState::Connecting);

            // Blocking spin-wait for TCP ESTABLISHED; pump NIC and send heartbeats.
            let mut spin: u32 = 0;
            loop {
                device.pump_rx();
                iface.poll(now_instant(), device, sockets);
                if spin % 200 == 0 { sys_heartbeat(500); }
                match sockets.get_mut::<tcp::Socket>(handle).state() {
                    tcp::State::Established => break,
                    tcp::State::Closed | tcp::State::CloseWait => {
                        table.remove(cap_id); sockets.remove(handle); sys_send(sender, &[0u8; 8]); return;
                    }
                    _ => {}
                }
                spin += 1;
                if spin > 20_000_000 { table.remove(cap_id); sockets.remove(handle); sys_send(sender, &[0u8; 8]); return; }
                core::hint::spin_loop();
            }
            table.set_state(cap_id, SocketState::Connected);

            let sockets_ptr = sockets as *mut SocketSet<'_> as *mut ();
            // SAFETY: pointers remain valid for the duration of handshake(); single-threaded cell.
            unsafe {
                crate::tls::transport::set_tls_context(
                    iface as *mut Interface, device as *mut VirtioNetDevice, sockets_ptr,
                );
            }
            match unsafe { TlsSocketEntry::handshake(handle, hostname) } {
                Ok(entry) => {
                    tls_table.insert(cap_id, entry);
                    sys_send(sender, &cap_id.to_le_bytes());
                }
                Err(e) => {
                    // Distinguish a verification REJECT from a transport timeout/drop:
                    // both collapse to cap 0 on the wire, but the log must be falsifiable
                    // (a timed-out handshake must not be mistaken for "MITM blocked").
                    let msg = match e {
                        embedded_tls::TlsError::InvalidCertificate
                        | embedded_tls::TlsError::InvalidCertificateEntry
                        | embedded_tls::TlsError::InvalidSignature => {
                            "[net/tls] connect REJECTED — certificate verification failed \
                             (untrusted CA / expired / hostname mismatch / tampered)"
                        }
                        embedded_tls::TlsError::Io(_) | embedded_tls::TlsError::IoError => {
                            "[net/tls] connect failed — transport I/O (timeout or connection drop)"
                        }
                        _ => "[net/tls] connect failed — TLS handshake error (other)",
                    };
                    let _ = ostd::syscall::sys_log(msg);
                    table.remove(cap_id);
                    sockets.remove(handle);
                    sys_send(sender, &[0u8; 8]);
                }
            }
        }

        TLS_SEND_OP => {
            // payload: [data:*]  reply: [bytes_written:4 LE]
            let sockets_ptr = sockets as *mut SocketSet<'_> as *mut ();
            let result = tls_table.get_mut(&cap).map(|entry|
                // SAFETY: pointers valid for duration of send; single-threaded cell.
                unsafe { entry.send(payload, iface as *mut _, device as *mut _, sockets_ptr) }
            );
            match result {
                Some(Ok(n)) => { sys_send(sender, &(n as u32).to_le_bytes()); }
                _           => { sys_send(sender, &0u32.to_le_bytes()); }
            }
        }

        TLS_RECV_OP => {
            // payload: [buf_len:4 LE]  reply: [data:*] or empty
            let buf_len = if payload.len() >= 4 {
                u32::from_le_bytes(payload[0..4].try_into().unwrap_or([0; 4])) as usize
            } else { 512 }.min(4096);
            let sockets_ptr = sockets as *mut SocketSet<'_> as *mut ();
            let result = tls_table.get_mut(&cap).map(|entry| {
                let mut data = alloc::vec![0u8; buf_len];
                // SAFETY: pointers valid for duration of recv; single-threaded cell.
                let r = unsafe { entry.recv(&mut data, iface as *mut _, device as *mut _, sockets_ptr) };
                (data, r)
            });
            match result {
                Some((data, Ok(n))) => { sys_send(sender, &data[..n]); }
                _                   => { sys_send(sender, &[]); }
            }
        }

        _ => { sys_send(sender, &[]); }
    }
}
