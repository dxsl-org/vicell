use api::config::ViConfig;
use api::ipc::{ConfigRequest, ConfigResponse, IPC_BUF_SIZE};
use ostd::prelude::*;

/// Client for the Config service.
///
/// Uses typed postcard IPC (`ConfigRequest` / `ConfigResponse`) matching the
/// config service v0.3 protocol.  Resolves the live Config endpoint via the
/// Service Registry on each call, so it transparently reconnects when the
/// supervisor respawns Config.
///
/// # Safety invariant
/// `resp_buf` is accessed exclusively through `&self` via `UnsafeCell`.
/// `ConfigClient` is only used from the shell's single thread.  `Sync` is
/// manually implemented for that reason; do not share across threads.
pub struct ConfigClient {
    // Stores the last received response so `get()` can return `&'self str`.
    // SAFETY: single-threaded cell; no concurrent get()/set() calls possible.
    resp_buf: core::cell::UnsafeCell<[u8; IPC_BUF_SIZE]>,
}

// SAFETY: ConfigClient is used exclusively from the shell's single thread.
unsafe impl Sync for ConfigClient {}

impl ConfigClient {
    pub fn new() -> Self {
        Self {
            resp_buf: core::cell::UnsafeCell::new([0u8; IPC_BUF_SIZE]),
        }
    }

    fn endpoint() -> Option<usize> {
        for _ in 0..8 {
            if let Some(tid) = ostd::syscall::sys_lookup_service(api::syscall::service::CONFIG) {
                return Some(tid);
            }
            ostd::task::yield_now();
        }
        None
    }
}

impl Default for ConfigClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ViConfig for ConfigClient {
    fn get(&self, key: &str) -> ViResult<&str> {
        let sid = Self::endpoint().ok_or(ViError::IO)?;

        let mut req_buf = [0u8; IPC_BUF_SIZE];
        let req = ConfigRequest::Get(key);
        let encoded = api::ipc::encode(&req, &mut req_buf).map_err(|_| ViError::IO)?;

        if let ostd::syscall::SyscallResult::Ok(_) = ostd::syscall::sys_send(sid, encoded) {
            // SAFETY: resp_buf is owned by self; no concurrent access (single-threaded cell).
            // We fill it here and return a &str sub-slice that borrows from self.resp_buf,
            // giving it the lifetime of &self — valid because resp_buf lives as long as self.
            let resp_buf = unsafe { &mut *self.resp_buf.get() };
            match ostd::syscall::sys_recv(0, resp_buf) {
                ostd::syscall::SyscallResult::Ok(sender) if sender == sid => {
                    match api::ipc::decode::<ConfigResponse>(resp_buf) {
                        Ok(ConfigResponse::Value(val)) => {
                            // val borrows from resp_buf which lives as long as self.
                            // SAFETY: We extend the lifetime from the buf borrow to 'self.
                            // The buf is not reused until the next get() call (single-threaded).
                            let extended: &str = unsafe { &*(val as *const str) };
                            Ok(extended)
                        }
                        Ok(ConfigResponse::NotFound) => Err(ViError::NotFound),
                        _ => Err(ViError::IO),
                    }
                }
                _ => Err(ViError::IO),
            }
        } else {
            Err(ViError::IO)
        }
    }

    fn set(&mut self, key: &str, value: &str) -> ViResult<()> {
        let sid = Self::endpoint().ok_or(ViError::IO)?;

        let mut req_buf = [0u8; IPC_BUF_SIZE];
        let req = ConfigRequest::Set { key, value };
        let encoded = api::ipc::encode(&req, &mut req_buf).map_err(|_| ViError::IO)?;

        ostd::syscall::sys_send(sid, encoded);

        let mut ack = [0u8; 64];
        ostd::syscall::sys_recv(0, &mut ack);
        Ok(())
    }
}
