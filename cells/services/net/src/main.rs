#![no_std]
#![no_main]

//! Net Service Cell.
//!
//! Drives a smoltcp TCP/IPv4 stack backed by the kernel VirtIO net driver.
//! Provides BSD-style socket IPC for consumer cells via typed postcard messages
//! (`api::ipc::NetRequest`/`NetResponse`).  Legacy TLS raw opcodes (0x30–0x32)
//! from `ostd::tls` are handled by the raw fallback in `handlers`.

extern crate alloc;

// Declares network capability; the kernel grants NetworkCap at spawn.
api::declare_manifest!(block_io = false, network = true, spawn = false);

// Narrow syscall allowlist -- kernel enforces this at dispatch (Phase 27).
api::declare_syscalls![
    Send, Recv, TryRecv, Reply, Log, Heartbeat, LookupService,
    NetTx, NetRx, GetTime,
    StateStash, StateRestore,
    GetRandom, WaitForEvent,
];

mod dhcp;
mod handlers;
mod interface;
mod poll_driver;
mod socket_state;
mod socket_table;
mod tls;

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU16, Ordering};
use dhcp::{add_dhcp_socket, poll_dhcp, DhcpState};
use interface::VirtioNetDevice;
use api::syscall::events::NET_RX;
use ostd::io::println;
use ostd::syscall::{sys_get_time, sys_try_recv, sys_wait_for_event, SyscallResult};
use poll_driver::POLL_INTERVAL_MS;
use smoltcp::{
    iface::{Config, Interface, SocketSet, SocketStorage},
    time::Instant,
    wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr},
};
use socket_table::{SocketTable, MAX_SOCKETS};
use crate::tls::socket::TlsSocketEntry;

/// Fixed IPC receive buffer size; mirrors api::ipc::IPC_BUF_SIZE.
const IPC_BUF_SIZE: usize = 512;

/// Fixed MAC address for QEMU VirtIO NIC (locally administered, unicast).
const MAC: EthernetAddress = EthernetAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// Number of ticks between forced smoltcp polls (fallback when no IPC arrives).
const POLL_TICKS: u64 = POLL_INTERVAL_MS * 10_000; // 100ms @ 10 MHz mtime

/// Ephemeral local port counter for outbound TCP connections.
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

pub(crate) fn next_ephemeral_port() -> u16 {
    let p = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    if p >= 65534 {
        NEXT_PORT.store(49152, Ordering::Relaxed);
    }
    p
}

pub(crate) fn now_instant() -> Instant {
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
                handlers::handle_request(
                    &buf,
                    sender,
                    &mut iface,
                    &mut device,
                    &mut sockets,
                    &mut table,
                    &mut tls_table,
                    &local_ip,
                );
            }
            _ => {
                // Block until NIC RX fires or the smoltcp maintenance deadline.
                // UNIT TRAP: WaitForEvent takes SCHEDULER ticks (10 ms each),
                // while POLL_TICKS is mtime ticks (10 MHz) for sys_get_time
                // comparisons. Passing POLL_TICKS here meant a ~2.8 h park —
                // the 5 s heartbeat then killed net every cycle (restart loop).
                let timeout_ticks = POLL_INTERVAL_MS / 10; // 100 ms → 10 ticks
                sys_wait_for_event(NET_RX, timeout_ticks);
            }
        }
    }
}
