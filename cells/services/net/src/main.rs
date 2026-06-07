#![no_std]
#![no_main]
// #[no_mangle] on main() requires removing #![forbid(unsafe_code)] for the same
// reason as other Cell entry points.  All logic in submodules is unsafe-free.

//! Net Service Cell.
//!
//! Drives a smoltcp TCP/IPv4 stack backed by the kernel VirtIO net driver.
//! Provides BSD-style socket IPC for consumer cells.

extern crate alloc;

// Declares network capability; the kernel grants NetworkCap at spawn.
api::declare_manifest!(block_io = false, network = true, spawn = false);

// Narrow syscall allowlist -- kernel enforces this at dispatch (Phase 27).
api::declare_syscalls![
    Send, Recv, TryRecv, Reply, Log, Heartbeat, LookupService,
    NetTx, NetRx, GetTime,
    StateStash, StateRestore,
    GetRandom,
];

mod dhcp;
mod interface;
mod poll_driver;
mod socket_state;
mod socket_table;
mod tls;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU16, Ordering};
use dhcp::{add_dhcp_socket, poll_dhcp, DhcpState};
use interface::VirtioNetDevice;
use ostd::io::println;
use ostd::syscall::{sys_get_time, sys_send, sys_try_recv, SyscallResult};
use poll_driver::{cell_opcodes, decode_message, NetMessage, POLL_INTERVAL_MS};
use smoltcp::{
    iface::{Config, Interface, SocketSet, SocketStorage},
    socket::{tcp, udp},
    time::Instant,
    wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint},
};
use socket_state::SocketState;
use socket_table::{SocketTable, MAX_SOCKETS};
use crate::tls::socket::TlsSocketEntry;

/// Fixed IPC payload size; mirrors api::ipc::IPC_BUF_SIZE.
const IPC_BUF_SIZE: usize = 512;

/// Fixed MAC address for QEMU VirtIO NIC (locally administered, unicast).
const MAC: EthernetAddress = EthernetAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// Number of ticks between forced smoltcp polls (fallback when no IPC arrives).
const POLL_TICKS: u64 = POLL_INTERVAL_MS * 10_000; // 100ms @ 10 MHz mtime

/// Ephemeral local port counter for outbound TCP connections.
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

fn next_ephemeral_port() -> u16 {
    let p = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    if p >= 65534 {
        NEXT_PORT.store(49152, Ordering::Relaxed);
    }
    p
}

fn now_instant() -> Instant {
    Instant::from_micros((sys_get_time() / 10) as i64)
}

#[no_mangle]
pub fn main() {
    println("[net] Network Service v0.1: smoltcp + VirtIO net + DHCP");

    let mut device = VirtioNetDevice::new();
    let cfg = Config::new(HardwareAddress::Ethernet(MAC));
    let mut iface = Interface::new(cfg, &mut device, now_instant());
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::v4(0, 0, 0, 0), 0));
    });

    let mut socket_storage = [SocketStorage::EMPTY; MAX_SOCKETS];
    let mut sockets = SocketSet::new(&mut socket_storage[..]);
    let mut table = SocketTable::new();
    let mut tls_table: BTreeMap<u64, TlsSocketEntry> = BTreeMap::new();

    let dhcp_handle = add_dhcp_socket(&mut sockets);
    let mut dhcp_state = DhcpState::Pending;

    let mut buf = [0u8; IPC_BUF_SIZE];
    let mut last_poll_ticks = sys_get_time();
    let mut local_ip = [0u8; 4];

    println("[net] Starting DHCP...");

    loop {
        ostd::syscall::sys_heartbeat(500);
        device.pump_rx();

        if dhcp_state == DhcpState::Pending {
            dhcp_state =
                poll_dhcp(dhcp_handle, &mut iface, &mut sockets, &mut device, now_instant());
            if dhcp_state == DhcpState::Acquired {
                if let Some(smoltcp::wire::IpCidr::Ipv4(cidr)) =
                    iface.ip_addrs().iter().find(|a| matches!(a, smoltcp::wire::IpCidr::Ipv4(_)))
                {
                    local_ip.copy_from_slice(cidr.address().as_bytes());
                    let mut s = alloc::string::String::from("[net] IP address: ");
                    for (i, oct) in local_ip.iter().enumerate() {
                        if i > 0 { s.push('.'); }
                        let mut n = *oct as u32;
                        let mut digits = [0u8; 3];
                        let mut di = 3;
                        loop {
                            di -= 1;
                            digits[di] = b'0' + (n % 10) as u8;
                            n /= 10;
                            if n == 0 { break; }
                        }
                        for d in &digits[di..] { s.push(*d as char); }
                    }
                    println(&s);
                }
            }
        }

        let now = sys_get_time();
        if now.wrapping_sub(last_poll_ticks) >= POLL_TICKS {
            iface.poll(now_instant(), &mut device, &mut sockets);
            last_poll_ticks = now;
        }

        buf.fill(0);
        match sys_try_recv(0, &mut buf) {
            SyscallResult::Ok(sender) if sender > 0 => {
                let scan_len = buf
                    .iter()
                    .rposition(|&b| b != 0)
                    .map(|i| i + 1)
                    .unwrap_or(0)
                    .max(9);
                let msg_len = match buf[0] {
                    0x12 => scan_len.max(15),
                    0x14 => scan_len.max(13),
                    0x16 => scan_len.max(11),
                    0x17 => scan_len.max(11),
                    0x21 => scan_len.max(15),
                    0x22 => scan_len.max(13),
                    0x23 => scan_len.max(13),
                    0x24 => scan_len.max(13),
                    0x30 => scan_len.max(15),
                    0x32 => scan_len.max(13),
                    _    => scan_len,
                };
                handle_ipc(
                    &buf[..msg_len],
                    sender,
                    &mut device,
                    &mut iface,
                    &mut sockets,
                    &mut table,
                    &mut tls_table,
                    &local_ip,
                );
            }
            _ => { ostd::task::yield_now(); }
        }
    }
}

fn handle_ipc(
    buf: &[u8],
    sender: usize,
    device: &mut VirtioNetDevice,
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    tls_table: &mut BTreeMap<u64, TlsSocketEntry>,
    local_ip: &[u8; 4],
) {
    match decode_message(buf) {
        NetMessage::RxFrame(frame) => {
            device.push_rx(Box::from(frame));
            iface.poll(now_instant(), device, sockets);
        }
        NetMessage::CellRequest { opcode, cap, payload } => {
            iface.poll(now_instant(), device, sockets);
            handle_socket_syscall(opcode, cap, payload, sender, iface, device, sockets, table, tls_table, local_ip);
            iface.poll(now_instant(), device, sockets);
        }
        NetMessage::Unknown => {}
    }
}

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

#[allow(clippy::too_many_arguments)]
fn handle_socket_syscall(
    opcode: u8,
    cap: u64,
    payload: &[u8],
    sender: usize,
    iface: &mut Interface,
    device: &mut VirtioNetDevice,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    tls_table: &mut BTreeMap<u64, TlsSocketEntry>,
    local_ip: &[u8; 4],
) {
    match opcode {
        cell_opcodes::SOCKET_TCP => {
            let rx_buf = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
            let tx_buf = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
            let socket = tcp::Socket::new(rx_buf, tx_buf);
            let handle = sockets.add(socket);
            match table.insert(handle) {
                Ok(cap_id) => { sys_send(sender, &cap_id.to_le_bytes()); }
                Err(_)     => { sys_send(sender, &[0u8; 8]); }
            }
        }

        cell_opcodes::CLOSE => {
            if let Some(handle) = table.remove(cap) {
                sockets.remove(handle);
            }
            tls_table.remove(&cap);
            sys_send(sender, &[0u8]);
        }

        cell_opcodes::GET_LOCAL_IP => {
            sys_send(sender, local_ip);
        }

        cell_opcodes::CONNECT => {
            if table.is_udp(cap) { sys_send(sender, &[0x01]); return; }
            if payload.len() < 6 { sys_send(sender, &[0x01]); return; }
            match table.get_state(cap) {
                Some(SocketState::Created) => {}
                Some(_) | None => { sys_send(sender, &[0x01]); return; }
            }
            let addr = [payload[0], payload[1], payload[2], payload[3]];
            let port = u16::from_le_bytes([payload[4], payload[5]]);
            let remote = IpEndpoint::new(
                IpAddress::v4(addr[0], addr[1], addr[2], addr[3]),
                port,
            );
            let local_port = next_ephemeral_port();
            if let Some(handle) = table.get(cap) {
                let cx = iface.context();
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                match socket.connect(cx, remote, local_port) {
                    Ok(()) => {
                        table.set_state(cap, SocketState::Connecting);
                        iface.poll(now_instant(), device, sockets);
                        sys_send(sender, &[0x00]);
                    }
                    Err(_) => { sys_send(sender, &[0x01]); }
                }
            } else {
                sys_send(sender, &[0x01]);
            }
        }

        cell_opcodes::SEND => {
            if table.is_udp(cap) { sys_send(sender, &0u32.to_le_bytes()); return; }
            if table.get_state(cap) == Some(SocketState::Connecting) {
                if let Some(handle) = table.get(cap) {
                    let s = sockets.get_mut::<tcp::Socket>(handle);
                    if s.state() == tcp::State::Established {
                        table.set_state(cap, SocketState::Connected);
                    }
                }
            }
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if socket.can_send() {
                    let n = socket.send_slice(payload).unwrap_or(0);
                    sys_send(sender, &(n as u32).to_le_bytes());
                } else {
                    sys_send(sender, &0u32.to_le_bytes());
                }
            } else {
                sys_send(sender, &0u32.to_le_bytes());
            }
        }

        cell_opcodes::RECV => {
            if table.is_udp(cap) { sys_send(sender, &[]); return; }
            if table.get_state(cap) == Some(SocketState::Connecting) {
                if let Some(handle) = table.get(cap) {
                    let s = sockets.get_mut::<tcp::Socket>(handle);
                    if s.state() == tcp::State::Established {
                        table.set_state(cap, SocketState::Connected);
                    }
                }
            }
            let buf_len = if payload.len() >= 4 {
                u32::from_le_bytes(payload[0..4].try_into().unwrap_or([0; 4])) as usize
            } else { 512 };
            let buf_len = buf_len.min(4096);
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                let mut data = alloc::vec![0u8; buf_len];
                let n = if socket.can_recv() {
                    socket.recv_slice(&mut data).unwrap_or(0)
                } else { 0 };
                sys_send(sender, &data[..n]);
            } else {
                sys_send(sender, &[]);
            }
        }

        cell_opcodes::SOCKET_STATE => {
            if table.is_udp(cap) { sys_send(sender, &[0x00]); return; }
            let byte = match table.get(cap) {
                Some(handle) => {
                    let socket = sockets.get_mut::<tcp::Socket>(handle);
                    tcp_state_byte(socket.state())
                }
                None => 0x00,
            };
            sys_send(sender, &[byte]);
        }

        cell_opcodes::LISTEN => {
            if table.is_udp(cap) { sys_send(sender, &[0x01]); return; }
            if payload.len() < 2 { sys_send(sender, &[0x01]); return; }
            if table.get_state(cap) != Some(SocketState::Created) {
                sys_send(sender, &[0x01]); return;
            }
            let port = u16::from_le_bytes([payload[0], payload[1]]);
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                match socket.listen(port) {
                    Ok(()) => {
                        table.set_state(cap, SocketState::Listening);
                        table.set_listen_port(cap, port);
                        sys_send(sender, &[0x00]);
                    }
                    Err(_) => { sys_send(sender, &[0x01]); }
                }
            } else {
                sys_send(sender, &[0x01]);
            }
        }

        cell_opcodes::ACCEPT => {
            if table.is_udp(cap) { sys_send(sender, &[0xFF_u8; 8]); return; }
            if table.get_state(cap) != Some(SocketState::Listening) {
                sys_send(sender, &[0xFF_u8; 8]); return;
            }
            let handle = match table.get(cap) {
                Some(h) => h,
                None => { sys_send(sender, &[0xFF_u8; 8]); return; }
            };
            {
                let s = sockets.get_mut::<tcp::Socket>(handle);
                if s.state() != tcp::State::Established {
                    sys_send(sender, &[0xFF_u8; 8]); return;
                }
            }
            let listen_port = match table.get_listen_port(cap) {
                Some(p) => p,
                None => { sys_send(sender, &[0xFF_u8; 8]); return; }
            };
            match table.insert_with_state(handle, SocketState::Connected) {
                Ok(stream_cap) => {
                    let rx = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
                    let tx = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
                    let mut new_sock = tcp::Socket::new(rx, tx);
                    let _ = new_sock.listen(listen_port);
                    let new_handle = sockets.add(new_sock);
                    table.update_handle(cap, new_handle);
                    table.set_state(cap, SocketState::Listening);
                    table.set_listen_port(cap, listen_port);
                    sys_send(sender, &stream_cap.to_le_bytes());
                }
                Err(_) => { sys_send(sender, &[0xFF_u8; 8]); }
            }
        }

        cell_opcodes::SOCKET_UDP => {
            let rx = udp::PacketBuffer::new(
                alloc::vec![udp::PacketMetadata::EMPTY; 4],
                alloc::vec![0u8; 1024],
            );
            let tx = udp::PacketBuffer::new(
                alloc::vec![udp::PacketMetadata::EMPTY; 4],
                alloc::vec![0u8; 1024],
            );
            let handle = sockets.add(udp::Socket::new(rx, tx));
            match table.insert(handle) {
                Ok(cap_id) => {
                    table.mark_udp(cap_id);
                    sys_send(sender, &cap_id.to_le_bytes());
                }
                Err(_) => { sys_send(sender, &[0u8; 8]); }
            }
        }

        cell_opcodes::BIND => {
            if payload.len() < 2 { sys_send(sender, &[0xFF, 0xFF]); return; }
            if table.get_state(cap) != Some(SocketState::Created) {
                sys_send(sender, &[0xFF, 0xFF]); return;
            }
            let requested = u16::from_le_bytes([payload[0], payload[1]]);
            let port = if requested == 0 { next_ephemeral_port() } else { requested };
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<udp::Socket>(handle);
                match socket.bind(port) {
                    Ok(()) => {
                        table.set_state(cap, SocketState::Listening);
                        sys_send(sender, &port.to_le_bytes());
                    }
                    Err(_) => { sys_send(sender, &[0xFF, 0xFF]); }
                }
            } else {
                sys_send(sender, &[0xFF, 0xFF]);
            }
        }

        cell_opcodes::SENDTO => {
            if payload.len() < 6 { sys_send(sender, &0u32.to_le_bytes()); return; }
            let addr = IpAddress::v4(payload[0], payload[1], payload[2], payload[3]);
            let dst_port = u16::from_le_bytes([payload[4], payload[5]]);
            let data = &payload[6..];
            let endpoint = IpEndpoint::new(addr, dst_port);
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<udp::Socket>(handle);
                match socket.send_slice(data, endpoint) {
                    Ok(()) => {
                        iface.poll(now_instant(), device, sockets);
                        sys_send(sender, &(data.len() as u32).to_le_bytes());
                    }
                    Err(_) => { sys_send(sender, &0u32.to_le_bytes()); }
                }
            } else {
                sys_send(sender, &0u32.to_le_bytes());
            }
        }

        cell_opcodes::RECVFROM => {
            let buf_len = if payload.len() >= 4 {
                u32::from_le_bytes(payload[0..4].try_into().unwrap_or([0; 4])) as usize
            } else { 512 };
            let buf_len = buf_len.min(512);
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<udp::Socket>(handle);
                if socket.can_recv() {
                    let mut data = alloc::vec![0u8; buf_len];
                    match socket.recv_slice(&mut data) {
                        Ok((n, meta)) => {
                            let mut reply = alloc::vec![0u8; 6 + n];
                            let IpAddress::Ipv4(src_ip) = meta.endpoint.addr;
                            reply[0..4].copy_from_slice(src_ip.as_bytes());
                            reply[4..6].copy_from_slice(&meta.endpoint.port.to_le_bytes());
                            reply[6..6 + n].copy_from_slice(&data[..n]);
                            sys_send(sender, &reply);
                        }
                        Err(_) => { sys_send(sender, &[]); }
                    }
                } else {
                    sys_send(sender, &[]);
                }
            } else {
                sys_send(sender, &[]);
            }
        }

        cell_opcodes::JOIN_MULTICAST => {
            if payload.len() < 4 { sys_send(sender, &[0x01]); return; }
            let group = IpAddress::v4(payload[0], payload[1], payload[2], payload[3]);
            match iface.join_multicast_group(device, group, now_instant()) {
                Ok(_) => sys_send(sender, &[0x00]),
                Err(_) => sys_send(sender, &[0x01]),
            };
        }

        cell_opcodes::LEAVE_MULTICAST => {
            if payload.len() < 4 { sys_send(sender, &[0x01]); return; }
            let group = IpAddress::v4(payload[0], payload[1], payload[2], payload[3]);
            match iface.leave_multicast_group(device, group, now_instant()) {
                Ok(_) => sys_send(sender, &[0x00]),
                Err(_) => sys_send(sender, &[0x01]),
            };
        }

        // ── TLS ──────────────────────────────────────────────────────────────

        cell_opcodes::TLS_CONNECT => {
            // payload: [addr:4][port:2 LE][hostname:*]
            // Creates TCP socket, connects, then performs TLS 1.3 handshake.
            // Blocking -- acceptable for single-user G1 robot demo.
            // Reply: [cap_id:8 LE] on success, [0u8;8] on failure.
            if payload.len() < 6 { sys_send(sender, &[0u8; 8]); return; }
            let addr = [payload[0], payload[1], payload[2], payload[3]];
            let port = u16::from_le_bytes([payload[4], payload[5]]);
            let hostname = core::str::from_utf8(&payload[6..])
                .unwrap_or("")
                .trim_end_matches('\0');

            let rx_buf = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
            let tx_buf = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
            let tcp_sock = tcp::Socket::new(rx_buf, tx_buf);
            let handle = sockets.add(tcp_sock);
            let cap_id = match table.insert(handle) {
                Ok(c) => c,
                Err(_) => { sockets.remove(handle); sys_send(sender, &[0u8; 8]); return; }
            };

            let remote = IpEndpoint::new(
                IpAddress::v4(addr[0], addr[1], addr[2], addr[3]),
                port,
            );
            let local_port = next_ephemeral_port();
            {
                let cx = iface.context();
                let s = sockets.get_mut::<tcp::Socket>(handle);
                if s.connect(cx, remote, local_port).is_err() {
                    table.remove(cap_id);
                    sockets.remove(handle);
                    sys_send(sender, &[0u8; 8]);
                    return;
                }
            }
            table.set_state(cap_id, SocketState::Connecting);

            // Spin-wait for TCP ESTABLISHED; pump NIC and heartbeat in the loop.
            let mut spin: u32 = 0;
            loop {
                device.pump_rx();
                iface.poll(now_instant(), device, sockets);
                if spin % 200 == 0 { ostd::syscall::sys_heartbeat(500); }
                let st = sockets.get_mut::<tcp::Socket>(handle).state();
                match st {
                    tcp::State::Established => break,
                    tcp::State::Closed | tcp::State::CloseWait => {
                        table.remove(cap_id);
                        sys_send(sender, &[0u8; 8]);
                        return;
                    }
                    _ => {}
                }
                spin += 1;
                if spin > 20_000_000 {
                    table.remove(cap_id);
                    sys_send(sender, &[0u8; 8]);
                    return;
                }
                core::hint::spin_loop();
            }
            table.set_state(cap_id, SocketState::Connected);

            // SAFETY: iface/device/sockets are valid for the duration of handshake().
            let sockets_ptr = sockets as *mut SocketSet<'_> as *mut ();
            unsafe {
                crate::tls::transport::set_tls_context(
                    iface  as *mut Interface,
                    device as *mut VirtioNetDevice,
                    sockets_ptr,
                );
            }

            match unsafe { TlsSocketEntry::handshake(handle, hostname) } {
                Ok(entry) => {
                    tls_table.insert(cap_id, entry);
                    sys_send(sender, &cap_id.to_le_bytes());
                }
                Err(_) => {
                    table.remove(cap_id);
                    sys_send(sender, &[0u8; 8]);
                }
            }
        }

        cell_opcodes::TLS_SEND => {
            // payload: [data:*]  reply: [bytes_written:4 LE]
            if let Some(entry) = tls_table.get_mut(&cap) {
                let sockets_ptr = sockets as *mut SocketSet<'_> as *mut ();
                let result = unsafe {
                    entry.send(
                        payload,
                        iface  as *mut Interface,
                        device as *mut VirtioNetDevice,
                        sockets_ptr,
                    )
                };
                match result {
                    Ok(n) => { sys_send(sender, &(n as u32).to_le_bytes()); }
                    Err(_) => { sys_send(sender, &0u32.to_le_bytes()); }
                }
            } else {
                sys_send(sender, &0u32.to_le_bytes());
            }
        }

        cell_opcodes::TLS_RECV => {
            // payload: [buf_len:4 LE]  reply: [data:*] or empty
            let buf_len = if payload.len() >= 4 {
                u32::from_le_bytes(payload[0..4].try_into().unwrap_or([0; 4])) as usize
            } else { 512 };
            let buf_len = buf_len.min(4096);
            if let Some(entry) = tls_table.get_mut(&cap) {
                let mut data = alloc::vec![0u8; buf_len];
                let sockets_ptr = sockets as *mut SocketSet<'_> as *mut ();
                let result = unsafe {
                    entry.recv(
                        &mut data,
                        iface  as *mut Interface,
                        device as *mut VirtioNetDevice,
                        sockets_ptr,
                    )
                };
                match result {
                    Ok(n) => { sys_send(sender, &data[..n]); }
                    Err(_) => { sys_send(sender, &[]); }
                }
            } else {
                sys_send(sender, &[]);
            }
        }

        _ => { sys_send(sender, &[]); }
    }
}