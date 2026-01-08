#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

use ostd::prelude::*;
use api::config::ViConfig;
use alloc::collections::BTreeMap;
use ostd::io::println;
use core::cell::RefCell;

// Singleton storage
struct ConfigStore {
    map: BTreeMap<String, String>,
}

impl ConfigStore {
    fn new() -> Self {
        let mut map = BTreeMap::new();
        // Default values
        map.insert(String::from("PATH"), String::from("/bin"));
        map.insert(String::from("OS"), String::from("ViOS"));
        Self { map }
    }
}

struct ConfigService {
    store: RefCell<ConfigStore>,
}

unsafe impl Sync for ConfigService {}

impl ConfigService {
    fn new() -> Self {
        Self {
            store: RefCell::new(ConfigStore::new())
        }
    }
}

// Implement ViConfig trait (conceptual, but here we handle IPC loop)
impl ViConfig for ConfigService {
    fn get(&self, key: &str) -> ViResult<String> {
        let store = self.store.borrow();
        store.map.get(key).cloned().ok_or(ViError::NotFound)
    }

    fn set(&self, key: &str, value: &str) -> ViResult<()> {
        let mut store = self.store.borrow_mut();
        store.map.insert(String::from(key), String::from(value));
        Ok(())
    }
}

#[no_mangle]
pub fn main() {
    println("Config Service: Starting...");

    let service = ConfigService::new();

    // IPC Loop
    // Simplistic: We just yield.
    // In real implementation, this would loop on `sys_recv`.
    // Protocol:
    // Msg: [OpCode(1byte) | KeyLen(1byte) | Key... | Val...]
    // OpCodes: 1=Get, 2=Set

    let mut buf = [0u8; 256];
    loop {
        // Mocking IPC Recv via syscall (Block/Yield)
        // Since we don't have a stable `sys_recv` that blocks nicely in current ostd yet
        // (Kernel Recv returns 0 if blocked and we yield), we yield.

        match ostd::syscall::sys_recv(0, &mut buf) {
            ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
                // Handle Message
                // Simplest protocol:
                // Byte 0: 1=Get, 2=Set
                // Byte 1: Key Len
                // Byte 2..: Key
                // ... Value

                if buf[0] == 1 { // Get
                    let key_len = buf[1] as usize;
                    if let Ok(key) = core::str::from_utf8(&buf[2..2+key_len]) {
                        // println("Config: Get Request");
                        if let Ok(val) = service.get(key) {
                            // Reply with Value
                            ostd::syscall::sys_send(sender, val.as_bytes());
                        } else {
                            ostd::syscall::sys_send(sender, b""); // Empty/Error
                        }
                    }
                } else if buf[0] == 2 { // Set
                    let key_len = buf[1] as usize;
                    let val_len = buf[2] as usize; // Simplified protocol
                    if let Ok(key) = core::str::from_utf8(&buf[3..3+key_len]) {
                        if let Ok(val) = core::str::from_utf8(&buf[3+key_len..3+key_len+val_len]) {
                             let _ = service.set(key, val);
                             // println("Config: Set Request");
                             ostd::syscall::sys_send(sender, b"OK");
                        }
                    }
                }
            },
            _ => {
                ostd::task::yield_now();
            }
        }
    }
}
