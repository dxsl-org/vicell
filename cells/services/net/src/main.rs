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
mod socket_table;

use alloc::boxed::Box;
use dhcp::{add_dhcp_socket, poll_dhcp, DhcpState};
use interface::VirtioNetDevice;
use ostd::io::println;
use ostd::syscall::{sys_get_time, sys_recv, sys_send, SyscallResult};
use poll_driver::{cell_opcodes, decode_message, NetMessage, POLL_INTERVAL_MS};
use smoltcp::{
    iface::{Config, Interface, SocketSet, SocketStorage},
    socket::tcp,
    time::Instant,
    wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr},
};
use socket_table::SocketTable;

/// Fixed MAC address for QEMU VirtIO NIC (locally administered, unicast).
const MAC: EthernetAddress = EthernetAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// Maximum simultaneous sockets.
const MAX_SOCKETS: usize = 18;

/// Number of ticks between forced smoltcp polls (fallback when no IPC arrives).
const POLL_TICKS: u64 = POLL_INTERVAL_MS * 10_000; // 100ms @ 10 MHz mtime

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
        // ── DHCP until acquired ───────────────────────────────────────────────
        if dhcp_state == DhcpState::Pending {
            dhcp_state =
                poll_dhcp(dhcp_handle, &mut iface, &mut sockets, &mut device, now_instant());
        }

        // ── Forced periodic poll ──────────────────────────────────────────────
        let now = sys_get_time();
        if now.wrapping_sub(last_poll_ticks) >= POLL_TICKS {
            iface.poll(now_instant(), &mut device, &mut sockets);
            last_poll_ticks = now;
        }

        // ── Receive one IPC message ───────────────────────────────────────────
        match sys_recv(0, &mut buf) {
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
            handle_socket_syscall(opcode, cap, payload, sender, sockets, table, local_ip);
        }
        NetMessage::Unknown => {}
    }
}

/// Handle socket syscall from a consumer cell.
fn handle_socket_syscall(
    opcode: u8,
    cap: u64,
    payload: &[u8],
    sender: usize,
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
        cell_opcodes::CONNECT | cell_opcodes::SEND | cell_opcodes::RECV
        | cell_opcodes::BIND | cell_opcodes::LISTEN | cell_opcodes::ACCEPT
        | cell_opcodes::SOCKET_UDP => {
            // Stub: full data-path deferred to Phase 17 readline + shell integration.
            let _ = (cap, payload);
            sys_send(sender, &[0xFF]); // not-yet-implemented
        }
        _ => {
            sys_send(sender, &[]);
        }
    }
}
