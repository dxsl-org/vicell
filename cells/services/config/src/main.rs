#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

use alloc::collections::BTreeMap;
use api::hotswap::ViStateTransfer;
use api::ipc::{ConfigRequest, ConfigResponse, IPC_BUF_SIZE};
use ostd::io::println;
use ostd::prelude::*;

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, TryRecv, Log, Heartbeat, LookupService, StateStash, StateRestore];

// Singleton storage
struct ConfigStore {
    map: BTreeMap<String, String>,
}

impl ConfigStore {
    fn new() -> Self {
        let mut map = BTreeMap::new();
        map.insert(String::from("PATH"), String::from("/bin"));
        map.insert(String::from("OS"), String::from("ViCell"));
        Self { map }
    }
}

struct ConfigService {
    store: Mutex<ConfigStore>,
}

impl ConfigService {
    fn new() -> Self {
        Self {
            store: Mutex::new(ConfigStore::new()),
        }
    }
}

#[no_mangle]
pub fn main() {
    println("[config] Config Service v0.3 (typed IPC)");

    let service = ConfigService::new();
    let mut buf = [0u8; IPC_BUF_SIZE];
    let mut resp_buf = [0u8; IPC_BUF_SIZE];

    loop {
        match ostd::syscall::sys_recv(0, &mut buf) {
            ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
                let response = match api::ipc::decode::<ConfigRequest>(&buf) {
                    Ok(ConfigRequest::Get(key)) => {
                        let store = service.store.lock();
                        match store.map.get(key) {
                            Some(val) => {
                                // Encode value inline into resp_buf while holding the lock,
                                // so the &str borrow from store does not escape.
                                let r = ConfigResponse::Value(val.as_str());
                                let encoded = api::ipc::encode(&r, &mut resp_buf);
                                drop(store);
                                if let Ok(slice) = encoded {
                                    ostd::syscall::sys_send(sender, slice);
                                    continue;
                                }
                                ConfigResponse::Err(0xFF)
                            }
                            None => {
                                drop(store);
                                ConfigResponse::NotFound
                            }
                        }
                    }
                    Ok(ConfigRequest::Set { key, value }) => {
                        let mut store = service.store.lock();
                        store.map.insert(String::from(key), String::from(value));
                        ConfigResponse::Ok
                    }
                    Ok(ConfigRequest::Delete(key)) => {
                        let mut store = service.store.lock();
                        store.map.remove(key);
                        ConfigResponse::Ok
                    }
                    Ok(ConfigRequest::List) => {
                        // Build a newline-separated key list and send inline.
                        let store = service.store.lock();
                        let mut list = alloc::string::String::new();
                        for k in store.map.keys() {
                            if !list.is_empty() {
                                list.push('\n');
                            }
                            list.push_str(k);
                        }
                        drop(store);
                        let r = ConfigResponse::Keys(list.as_str());
                        if let Ok(slice) = api::ipc::encode(&r, &mut resp_buf) {
                            ostd::syscall::sys_send(sender, slice);
                        }
                        continue;
                    }
                    Err(_) => ConfigResponse::Err(0xFF),
                };

                if let Ok(slice) = api::ipc::encode(&response, &mut resp_buf) {
                    ostd::syscall::sys_send(sender, slice);
                }
            }
            _ => {
                ostd::task::yield_now();
            }
        }
    }
}

// ─── Hot-swap state transfer ──────────────────────────────────────────────────
//
// Wire format (little-endian):
//   [count: u32][key_len: u16][key bytes][val_len: u16][val bytes]...
//
// Schema version 1 is prepended as a u32 for forward compatibility.

const SCHEMA_VERSION: u32 = 1;

impl ViStateTransfer for ConfigStore {
    fn state_size(&self) -> usize {
        // version(4) + count(4) + per-entry overhead(4) + key+val bytes
        4 + 4 + self.map.iter().map(|(k, v)| 2 + k.len() + 2 + v.len()).sum::<usize>()
    }

    fn serialize_state(&self, buf: &mut [u8]) -> ViResult<usize> {
        let needed = self.state_size();
        if buf.len() < needed { return Err(ViError::InvalidArgument); }
        let mut pos = 0;
        buf[pos..pos+4].copy_from_slice(&SCHEMA_VERSION.to_le_bytes()); pos += 4;
        let count = self.map.len() as u32;
        buf[pos..pos+4].copy_from_slice(&count.to_le_bytes()); pos += 4;
        for (k, v) in &self.map {
            let kl = k.len() as u16;
            let vl = v.len() as u16;
            buf[pos..pos+2].copy_from_slice(&kl.to_le_bytes()); pos += 2;
            buf[pos..pos+k.len()].copy_from_slice(k.as_bytes()); pos += k.len();
            buf[pos..pos+2].copy_from_slice(&vl.to_le_bytes()); pos += 2;
            buf[pos..pos+v.len()].copy_from_slice(v.as_bytes()); pos += v.len();
        }
        Ok(pos)
    }

    fn deserialize_state(&mut self, buf: &[u8]) -> ViResult<()> {
        if buf.len() < 8 { return Err(ViError::InvalidInput); }
        let _version = u32::from_le_bytes([buf[0],buf[1],buf[2],buf[3]]);
        let count = u32::from_le_bytes([buf[4],buf[5],buf[6],buf[7]]) as usize;
        let mut pos = 8usize;
        self.map.clear();
        for _ in 0..count {
            if pos + 2 > buf.len() { return Err(ViError::InvalidInput); }
            let kl = u16::from_le_bytes([buf[pos], buf[pos+1]]) as usize; pos += 2;
            if pos + kl > buf.len() { return Err(ViError::InvalidInput); }
            let key = core::str::from_utf8(&buf[pos..pos+kl]).map_err(|_| ViError::InvalidInput)?;
            pos += kl;
            if pos + 2 > buf.len() { return Err(ViError::InvalidInput); }
            let vl = u16::from_le_bytes([buf[pos], buf[pos+1]]) as usize; pos += 2;
            if pos + vl > buf.len() { return Err(ViError::InvalidInput); }
            let val = core::str::from_utf8(&buf[pos..pos+vl]).map_err(|_| ViError::InvalidInput)?;
            pos += vl;
            self.map.insert(String::from(key), String::from(val));
        }
        Ok(())
    }
}
