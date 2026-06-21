//! MQTT 3.1.1 QoS-0 telemetry over TCP-IPC.
//!
//! All functions are best-effort: they return without panicking when the net
//! service is absent or the broker is unreachable.

extern crate alloc;

use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;
use ostd::io::println;
use ostd::syscall::{sys_lookup_service, sys_recv_timeout, sys_send, SyscallResult};

// 200ms per IPC call (system_ticks() runs at 100 Hz = 10ms per tick).
// Each sys_recv_timeout gets a fresh 20-tick (200ms) relative timeout.
// In test environments without a broker this bounds each MQTT cycle to ~2.4s.
const RECV_TIMEOUT_SOFT_TICKS: u64 = 20;

const MQTT_ADDR: [u8; 4] = [10, 0, 2, 2]; // QEMU SLIRP gateway
const MQTT_PORT: u16 = 1883;

/// Publish `payload` to the `vios/robot` topic.
///
/// Opens a fresh TCP connection per call (fire-and-forget QoS-0). Gracefully
/// skips if the net service is not registered or the broker is unreachable.
pub fn publish_telemetry(payload: &str) {
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => {
            println("[robot-demo] no net service — skipping MQTT");
            return;
        }
    };

    let mut buf = [0u8; IPC_BUF_SIZE];
    let cap: u32 = {
        let len = api::ipc::encode(
            &NetRequest::TcpConnect { addr: MQTT_ADDR, port: MQTT_PORT },
            &mut buf,
        )
        .map(|b| b.len())
        .unwrap_or(0);
        sys_send(net_ep, &buf[..len]);
        let mut r = [0u8; IPC_BUF_SIZE];
        match sys_recv_timeout(net_ep, &mut r, RECV_TIMEOUT_SOFT_TICKS) {
            SyscallResult::Ok(0) => {
                println("[robot-demo] MQTT connect: net service timeout");
                return;
            }
            SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&r) {
                Ok(NetResponse::CapId(c)) => c,
                _ => {
                    println("[robot-demo] MQTT broker unreachable");
                    return;
                }
            },
            _ => {
                println("[robot-demo] MQTT connect failed");
                return;
            }
        }
    };

    if !mqtt_handshake(net_ep, cap) {
        println("[robot-demo] CONNACK failed — closing");
        close_cap(net_ep, cap);
        return;
    }

    mqtt_publish(net_ep, cap, "vios/robot", payload);
    close_cap(net_ep, cap);
    println("[robot-demo] MQTT telemetry published");
}

/// Send MQTT CONNECT, retry up to 5 times until the TCP handshake completes,
/// then accumulate data until CONNACK arrives. Returns true on `CONNACK RC=0`.
fn mqtt_handshake(net_ep: usize, cap: u32) -> bool {
    const PKTLEN: usize = 18;
    let connect_pkt: [u8; PKTLEN] = [
        0x10, 0x10, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x04,
        b'v', b'i', b'o', b's',
    ];
    let mut connect_sent = false;
    let mut connack = [0u8; 64];
    let mut n_accum = 0usize;

    for _ in 0..5usize {
        if !connect_sent {
            connect_sent = tcp_try_send(net_ep, cap, &connect_pkt) > 0;
        }
        if connect_sent {
            let mut tmp = [0u8; 64];
            let n = tcp_recv_once(net_ep, cap, &mut tmp);
            if n > 0 && n_accum + n <= connack.len() {
                connack[n_accum..n_accum + n].copy_from_slice(&tmp[..n]);
                n_accum += n;
            }
            if n_accum >= 4 {
                return connack[0] == 0x20 && connack[3] == 0x00;
            }
        }
        ostd::task::yield_now();
    }
    false
}

fn mqtt_publish(net_ep: usize, cap: u32, topic: &str, payload: &str) {
    let tb = topic.as_bytes();
    let pb = payload.as_bytes();
    let remaining = 2 + tb.len() + pb.len();
    let mut pkt = [0u8; 340];
    let mut rl = [0u8; 4];
    let rl_len = encode_remaining_len(remaining, &mut rl);
    if 1 + rl_len + remaining > pkt.len() {
        println("[robot-demo] mqtt_publish: packet exceeds 340 bytes");
        return;
    }
    pkt[0] = 0x30;
    pkt[1..1 + rl_len].copy_from_slice(&rl[..rl_len]);
    let mut p = 1 + rl_len;
    pkt[p] = (tb.len() >> 8) as u8;
    pkt[p + 1] = tb.len() as u8;
    p += 2;
    pkt[p..p + tb.len()].copy_from_slice(tb);
    p += tb.len();
    pkt[p..p + pb.len()].copy_from_slice(pb);
    p += pb.len();
    tcp_send_raw(net_ep, cap, &pkt[..p]);
}

/// Send data and return bytes accepted (0 = socket not yet Established or timeout).
fn tcp_try_send(net_ep: usize, cap: u32, data: &[u8]) -> usize {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpSend { cap_id: cap, data }, &mut buf)
        .map(|b| b.len())
        .unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    match sys_recv_timeout(net_ep, &mut r, RECV_TIMEOUT_SOFT_TICKS) {
        SyscallResult::Ok(0) => 0,
        SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&r) {
            Ok(NetResponse::Data(b)) if b.len() >= 4 => {
                let mut arr = [0u8; 4];
                arr.copy_from_slice(&b[0..4]);
                u32::from_le_bytes(arr) as usize
            }
            _ => 0,
        },
        _ => 0,
    }
}

/// Fire-and-forget send; used after Established is confirmed.
fn tcp_send_raw(net_ep: usize, cap: u32, data: &[u8]) {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpSend { cap_id: cap, data }, &mut buf)
        .map(|b| b.len())
        .unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv_timeout(net_ep, &mut r, RECV_TIMEOUT_SOFT_TICKS);
}

fn tcp_recv_once(net_ep: usize, cap: u32, out: &mut [u8; 64]) -> usize {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id: cap, buf_len: 64 },
        &mut buf,
    )
    .map(|b| b.len())
    .unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    match sys_recv_timeout(net_ep, &mut r, RECV_TIMEOUT_SOFT_TICKS) {
        SyscallResult::Ok(0) => 0,
        SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&r) {
            Ok(NetResponse::Data(b)) if !b.is_empty() => {
                let n = b.len().min(64);
                out[..n].copy_from_slice(&b[..n]);
                n
            }
            _ => 0,
        },
        _ => 0,
    }
}

fn close_cap(net_ep: usize, cap: u32) {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpClose { cap_id: cap }, &mut buf)
        .map(|b| b.len())
        .unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv_timeout(net_ep, &mut r, RECV_TIMEOUT_SOFT_TICKS);
}

fn encode_remaining_len(mut n: usize, out: &mut [u8; 4]) -> usize {
    let mut i = 0;
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out[i] = b;
        i += 1;
        if n == 0 || i == 4 {
            break;
        }
    }
    i
}
