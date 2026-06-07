#![no_std]
#![no_main]

//! Reference robot demo — G1 graduation criterion 8.
//!
//! Runs a sensor→compute→actuator GPIO loop then publishes
//! one MQTT telemetry message via the net service.

extern crate alloc;

use api::ipc::{IPC_BUF_SIZE, NetRequest, NetResponse};
use api::syscall::service;
use api::declare_manifest;
use driver_gpio::Pl061Gpio;
use hal_gpio::{PinDir, ViGpio};
use ostd::io::println;
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send, SyscallResult};
use types::ViError;

declare_manifest!(block_io = false, network = true, spawn = false, gpio = true, uart = false);
api::declare_syscalls![Send, Recv, Log, LookupService, Heartbeat];

const SENSOR_PIN:   u8     = 2;
const ACTUATOR_PIN: u8     = 3;
const LOOP_CYCLES:  u32    = 5;
const MQTT_ADDR:   [u8; 4] = [10, 0, 2, 2]; // QEMU SLIRP gateway
const MQTT_PORT:    u16    = 1883;

#[no_mangle]
pub fn main() {
    println("[robot-demo] ViCell reference robot demo (G1 graduation criterion 8)");

    let mut gpio = match Pl061Gpio::open() {
        Ok(g) => g,
        Err(ViError::PermissionDenied) => {
            println("[robot-demo] GPIO not available — running simulation");
            simulate_loop();
            return;
        }
        Err(e) => {
            let msg = alloc::format!("[robot-demo] GPIO open error: {:?}", e);
            println(&msg);
            return;
        }
    };

    if gpio.set_direction(SENSOR_PIN, PinDir::Input).is_err()
        || gpio.set_direction(ACTUATOR_PIN, PinDir::Output).is_err()
    {
        println("[robot-demo] pin config failed");
        return;
    }

    println("[robot-demo] GPIO configured; starting control loop");

    for i in 0..LOOP_CYCLES {
        control_step(&mut gpio, i);
        ostd::task::yield_now();
    }

    let _ = gpio.write_pin(ACTUATOR_PIN, false);
    publish_telemetry("gpio", "loop_done", LOOP_CYCLES);
    println("[robot-demo] demo complete");
}

/// One sensor→compute→actuator cycle.
///
/// QEMU GPIO pin reads always return LOW (no physical sensor), so a
/// tick-based synthetic value exercises both actuator states while
/// still driving the real GPIO write path on aarch64.
fn control_step(gpio: &mut Pl061Gpio, tick: u32) {
    let sensor_high = tick % 2 == 0; // even ticks = HIGH
    let _ = gpio.write_pin(ACTUATOR_PIN, sensor_high);
    let msg = alloc::format!(
        "[robot-demo] tick={} sensor={} actuator={}",
        tick,
        if sensor_high { "HIGH" } else { "LOW" },
        if sensor_high { "ON" } else { "OFF" },
    );
    println(&msg);
}

fn simulate_loop() {
    for i in 0..LOOP_CYCLES {
        let sensor = i % 2 == 0;
        let msg = alloc::format!(
            "[robot-demo] (sim) tick={} sensor={} actuator={}",
            i,
            if sensor { "HIGH" } else { "LOW" },
            if sensor { "ON" } else { "OFF" },
        );
        println(&msg);
        ostd::task::yield_now();
    }
    publish_telemetry("sim", "sim_done", LOOP_CYCLES);
}

/// Publish a JSON telemetry event via MQTT. Gracefully skips if net is absent
/// or the broker is unreachable.
fn publish_telemetry(suffix: &str, event: &str, count: u32) {
    let net_ep = match sys_lookup_service(service::NET) {
        Some(ep) => ep,
        None => { println("[robot-demo] no net service — skipping MQTT"); return; }
    };

    let mut buf = [0u8; IPC_BUF_SIZE];
    let cap: u32 = {
        let len = api::ipc::encode(
            &NetRequest::TcpConnect { addr: MQTT_ADDR, port: MQTT_PORT },
            &mut buf,
        ).map(|b| b.len()).unwrap_or(0);
        sys_send(net_ep, &buf[..len]);
        let mut r = [0u8; IPC_BUF_SIZE];
        match sys_recv(net_ep, &mut r) {
            SyscallResult::Ok(_) => match api::ipc::decode::<NetResponse>(&r) {
                Ok(NetResponse::CapId(c)) => c,
                _ => { println("[robot-demo] MQTT broker unreachable"); return; }
            },
            _ => { println("[robot-demo] MQTT connect failed"); return; }
        }
    };

    if !mqtt_handshake(net_ep, cap) {
        println("[robot-demo] CONNACK failed — closing");
        close_cap(net_ep, cap);
        return;
    }

    let payload = alloc::format!(
        r#"{{"device":"robot-demo/{}","event":"{}","count":{}}}"#,
        suffix, event, count
    );
    mqtt_publish(net_ep, cap, "vios/robot", &payload);
    close_cap(net_ep, cap);
    println("[robot-demo] MQTT telemetry published");
}

/// Send MQTT CONNECT, retry until the TCP handshake completes (socket
/// transitions from Connecting → Established before CONNECT is accepted),
/// then accumulate data until CONNACK arrives.
fn mqtt_handshake(net_ep: usize, cap: u32) -> bool {
    const PKTLEN: usize = 18;
    let connect_pkt: [u8; PKTLEN] = [
        0x10, 0x10,
        0x00, 0x04, b'M', b'Q', b'T', b'T',
        0x04, 0x02, 0x00, 0x3C,
        0x00, 0x04, b'v', b'i', b'o', b's',
    ];
    let mut connect_sent = false;
    let mut connack = [0u8; 64];
    let mut n_accum = 0usize;

    for _ in 0..1000usize {
        if !connect_sent {
            // tcp_try_send returns 0 while socket is still Connecting
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
    pkt[p]     = (tb.len() >> 8) as u8;
    pkt[p + 1] = tb.len() as u8;
    p += 2;
    pkt[p..p + tb.len()].copy_from_slice(tb); p += tb.len();
    pkt[p..p + pb.len()].copy_from_slice(pb); p += pb.len();
    tcp_send_raw(net_ep, cap, &pkt[..p]);
}

/// Send data and return bytes accepted (0 = socket not yet Established).
fn tcp_try_send(net_ep: usize, cap: u32, data: &[u8]) -> usize {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(&NetRequest::TcpSend { cap_id: cap, data }, &mut buf)
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    match sys_recv(net_ep, &mut r) {
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
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv(net_ep, &mut r);
}

fn tcp_recv_once(net_ep: usize, cap: u32, out: &mut [u8; 64]) -> usize {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let len = api::ipc::encode(
        &NetRequest::TcpRecv { cap_id: cap, buf_len: 64 },
        &mut buf,
    ).map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    match sys_recv(net_ep, &mut r) {
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
        .map(|b| b.len()).unwrap_or(0);
    sys_send(net_ep, &buf[..len]);
    let mut r = [0u8; IPC_BUF_SIZE];
    let _ = sys_recv(net_ep, &mut r);
}

fn encode_remaining_len(mut n: usize, out: &mut [u8; 4]) -> usize {
    let mut i = 0;
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 { b |= 0x80; }
        out[i] = b;
        i += 1;
        if n == 0 || i == 4 { break; }
    }
    i
}
