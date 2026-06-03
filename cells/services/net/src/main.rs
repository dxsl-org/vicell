#![no_std]
#![no_main]
// #[no_mangle] on main() requires removing #![forbid(unsafe_code)] for the same
// reason as other Cell entry points.  All logic in submodules is unsafe-free.

//! Net Service Cell.
//!
//! Drives a smoltcp TCP/IPv4 stack backed by the kernel VirtIO net driver.
//! Provides BSD-style socket IPC for consumer cells.

extern crate alloc;

mod dhcp;
mod interface;
mod poll_driver;
mod socket_state;
mod socket_table;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU16, Ordering};
use dhcp::{add_dhcp_socket, poll_dhcp, DhcpState};
use interface::VirtioNetDevice;
use ostd::io::println;
use ostd::syscall::{sys_get_time, sys_send, sys_try_recv, SyscallResult};
use poll_driver::{cell_opcodes, decode_message, NetMessage, POLL_INTERVAL_MS};
use smoltcp::{
    iface::{Config, Interface, SocketSet, SocketStorage},
    socket::tcp,
    time::Instant,
    wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint},
};
use socket_state::SocketState;
use socket_table::SocketTable;

/// Fixed MAC address for QEMU VirtIO NIC (locally administered, unicast).
const MAC: EthernetAddress = EthernetAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// Maximum simultaneous sockets.
const MAX_SOCKETS: usize = 18;

/// Number of ticks between forced smoltcp polls (fallback when no IPC arrives).
const POLL_TICKS: u64 = POLL_INTERVAL_MS * 10_000; // 100ms @ 10 MHz mtime

/// Ephemeral local port counter for outbound TCP connections.
///
/// Wraps in the IANA ephemeral range (49152–65534). Single-core kernel makes
/// Relaxed ordering safe — no concurrent writers exist.
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

fn next_ephemeral_port() -> u16 {
    let p = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    if p >= 65534 {
        NEXT_PORT.store(49152, Ordering::Relaxed);
    }
    p
}

fn now_instant() -> Instant {
    // Convert kernel ticks (10 MHz) to smoltcp Instant (microseconds).
    Instant::from_micros((sys_get_time() / 10) as i64)
}

#[no_mangle]
pub fn main() {
    println("[net] Network Service v0.1: smoltcp + VirtIO net + DHCP");

    // ── smoltcp setup ────────────────────────────────────────────────────────
    let mut device = VirtioNetDevice::new();
    let cfg = Config::new(HardwareAddress::Ethernet(MAC));
    let mut iface = Interface::new(cfg, &mut device, now_instant());
    // Initially no IP; DHCP will assign one.
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::v4(0, 0, 0, 0), 0));
    });

    // Fixed-size socket storage — array init is valid because SocketStorage::EMPTY is const.
    let mut socket_storage = [SocketStorage::EMPTY; MAX_SOCKETS];
    let mut sockets = SocketSet::new(&mut socket_storage[..]);
    let mut table = SocketTable::new();

    // Start DHCP.
    let dhcp_handle = add_dhcp_socket(&mut sockets);
    let mut dhcp_state = DhcpState::Pending;

    let mut buf = [0u8; 512];
    let mut last_poll_ticks = sys_get_time();
    let mut local_ip = [0u8; 4];

    println("[net] Starting DHCP...");

    loop {
        // ── Pull inbound frames from the kernel NIC ───────────────────────────
        // Without this the smoltcp stack never sees DHCP OFFER/ACK and stays
        // stuck in DISCOVER forever.
        device.pump_rx();

        // ── DHCP until acquired ───────────────────────────────────────────────
        if dhcp_state == DhcpState::Pending {
            dhcp_state =
                poll_dhcp(dhcp_handle, &mut iface, &mut sockets, &mut device, now_instant());
            // Cache the leased address octets for GET_LOCAL_IP queries.
            if dhcp_state == DhcpState::Acquired {
                if let Some(smoltcp::wire::IpCidr::Ipv4(cidr)) =
                    iface.ip_addrs().iter().find(|a| matches!(a, smoltcp::wire::IpCidr::Ipv4(_)))
                {
                    local_ip.copy_from_slice(cidr.address().as_bytes());
                    let mut s = alloc::string::String::from("[net] IP address: ");
                    for (i, oct) in local_ip.iter().enumerate() {
                        if i > 0 { s.push('.'); }
                        // u8 → decimal without std fmt machinery on the hot path.
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

        // ── Forced periodic poll ──────────────────────────────────────────────
        let now = sys_get_time();
        if now.wrapping_sub(last_poll_ticks) >= POLL_TICKS {
            iface.poll(now_instant(), &mut device, &mut sockets);
            last_poll_ticks = now;
        }

        // ── Receive one IPC message (non-blocking) ────────────────────────────
        // Must NOT block: the loop has to keep pumping RX frames and polling
        // DHCP. A blocking recv would park the cell forever (no IPC arrives
        // during DHCP) and the lease would never complete.
        match sys_try_recv(0, &mut buf) {
            SyscallResult::Ok(sender) if sender > 0 => {
                handle_ipc(
                    &buf,
                    sender,
                    &mut device,
                    &mut iface,
                    &mut sockets,
                    &mut table,
                    &mut local_ip,
                );
            }
            _ => {
                ostd::task::yield_now();
            }
        }
    }
}

/// Dispatch one IPC message.
fn handle_ipc(
    buf: &[u8; 512],
    sender: usize,
    device: &mut VirtioNetDevice,
    iface: &mut Interface,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    local_ip: &mut [u8; 4],
) {
    match decode_message(buf) {
        NetMessage::RxFrame(frame) => {
            device.push_rx(Box::from(frame));
            iface.poll(now_instant(), device, sockets);
        }
        NetMessage::CellRequest { opcode, cap, payload } => {
            // Advance smoltcp before and after the syscall so TCP state
            // transitions (SYN-SENT → ESTABLISHED) happen promptly.
            iface.poll(now_instant(), device, sockets);
            handle_socket_syscall(opcode, cap, payload, sender, iface, device, sockets, table, local_ip);
            iface.poll(now_instant(), device, sockets);
        }
        NetMessage::Unknown => {}
    }
}

/// Map a smoltcp TCP state to the 1-byte wire encoding consumers expect.
///
/// No wildcard arm: `tcp::State` is exhaustive in smoltcp 0.11 (11 variants,
/// no `#[non_exhaustive]`). A `_ =>` arm would be unreachable and fail
/// `clippy -D warnings`.
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

/// Handle socket syscall from a consumer cell.
fn handle_socket_syscall(
    opcode: u8,
    cap: u64,
    payload: &[u8],
    sender: usize,
    iface: &mut Interface,
    device: &mut VirtioNetDevice,
    sockets: &mut SocketSet<'_>,
    table: &mut SocketTable,
    local_ip: &[u8; 4],
) {
    match opcode {
        cell_opcodes::SOCKET_TCP => {
            // Create a TCP socket and return its CapId.
            let rx_buf = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
            let tx_buf = tcp::SocketBuffer::new(alloc::vec![0u8; 4096]);
            let socket = tcp::Socket::new(rx_buf, tx_buf);
            let handle = sockets.add(socket);
            match table.insert(handle) {
                Ok(cap_id) => {
                    sys_send(sender, &cap_id.to_le_bytes());
                }
                Err(_) => {
                    sys_send(sender, &[0u8; 8]); // 0 = error
                }
            }
        }
        cell_opcodes::CLOSE => {
            if let Some(handle) = table.remove(cap) {
                sockets.remove(handle);
            }
            sys_send(sender, &[0u8]); // ok
        }
        cell_opcodes::GET_LOCAL_IP => {
            sys_send(sender, local_ip);
        }
        cell_opcodes::CONNECT => {
            if payload.len() < 6 {
                sys_send(sender, &[0x01]);
                return;
            }
            // Guard against double-connect.
            match table.get_state(cap) {
                Some(SocketState::Created) => {}
                Some(_) | None => {
                    sys_send(sender, &[0x01]); // wrong state or unknown cap
                    return;
                }
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
                        // Flush the SYN immediately instead of waiting for the next
                        // periodic poll — reduces handshake latency by up to 100 ms.
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
            // Update Connecting → Connected if the handshake has completed.
            if table.get_state(cap) == Some(SocketState::Connecting) {
                if let Some(handle) = table.get(cap) {
                    let s = sockets.get_mut::<tcp::Socket>(handle);
                    if s.state() == tcp::State::Established {
                        table.set_state(cap, SocketState::Connected);
                    }
                }
            }
            let data = payload;
            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if socket.can_send() {
                    let n = socket.send_slice(data).unwrap_or(0);
                    sys_send(sender, &(n as u32).to_le_bytes());
                } else {
                    sys_send(sender, &0u32.to_le_bytes()); // not ready yet
                }
            } else {
                sys_send(sender, &0u32.to_le_bytes());
            }
        }
        cell_opcodes::RECV => {
            // Update Connecting → Connected if the handshake has completed.
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
            } else {
                512
            };
            let buf_len = buf_len.min(4096);

            if let Some(handle) = table.get(cap) {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                let mut data = alloc::vec![0u8; buf_len];
                let n = if socket.can_recv() {
                    socket.recv_slice(&mut data).unwrap_or(0)
                } else {
                    0
                };
                sys_send(sender, &data[..n]); // 0-byte reply = no data yet
            } else {
                sys_send(sender, &[]);
            }
        }
        cell_opcodes::SOCKET_STATE => {
            // Read-only: must NOT mutate table state.
            let byte = match table.get(cap) {
                Some(handle) => {
                    let socket = sockets.get_mut::<tcp::Socket>(handle);
                    tcp_state_byte(socket.state())
                }
                None => 0x00, // unknown cap == effectively closed
            };
            sys_send(sender, &[byte]);
        }
        cell_opcodes::BIND | cell_opcodes::LISTEN | cell_opcodes::ACCEPT
        | cell_opcodes::SOCKET_UDP => {
            let _ = (cap, payload);
            sys_send(sender, &[0xFF]); // not-yet-implemented
        }
        _ => {
            sys_send(sender, &[]);
        }
    }
}
