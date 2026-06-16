// SPDX-License-Identifier: MPL-2.0

//! Service-side message dispatch for ViCell cells.
//!
//! [`MessageHandler`] lets a service cell declare a typed request/response contract
//! using Generic Associated Types (stable Rust ≥ 1.65). The handler trait is then
//! driven by [`run_service`], which owns the receive buffer, decodes each incoming
//! message, calls the handler, and sends the response — all in a tight loop.
//!
//! # Example (VFS service)
//! ```no_run
//! use ostd::dispatch::{MessageHandler, run_service};
//! use api::ipc::{VfsRequest, VfsResponse};
//!
//! struct VfsHandler;
//!
//! impl MessageHandler for VfsHandler {
//!     type Request<'de> = VfsRequest<'de>;
//!     type Response = VfsResponse<'static>;
//!
//!     fn handle<'de>(&mut self, req: Self::Request<'de>, _sender: usize) -> Self::Response {
//!         match req {
//!             VfsRequest::Stat(path) => VfsResponse::Stat { size: 0, is_dir: false },
//!             _ => VfsResponse::Error("unimplemented"),
//!         }
//!     }
//! }
//!
//! run_service(VfsHandler, 0);  // never returns
//! ```

use crate::{syscall, ViError};
use api::ipc::IPC_BUF_SIZE;

/// Typed service message handler with GAT-based request lifetime.
///
/// Implement this on your service struct. `Request<'de>` may borrow from the
/// receive buffer (e.g. `VfsRequest<'de>` borrows path strings), while `Response`
/// must be fully owned (it is serialized before the buffer is reused).
///
/// # Lifetimes
/// The `'de` lifetime in `Request<'de>: Deserialize<'de>` ties the decoded request
/// to the receive buffer for that call. The buffer is not reused until `handle`
/// returns, so borrowed data in the request remains valid for the entire handler call.
pub trait MessageHandler {
    /// Decoded request type, possibly borrowing from the receive buffer.
    type Request<'de>: serde::Deserialize<'de>;
    /// Owned response type — serialized and sent back to the caller.
    type Response: serde::Serialize;

    /// Process one message from `sender_tid`, return the response.
    ///
    /// Panicking here kills the cell. Prefer returning an error variant.
    fn handle<'de>(&mut self, req: Self::Request<'de>, sender_tid: usize) -> Self::Response;
}

/// Run a service message loop driven by `handler`.
///
/// Blocks forever. On each iteration:
/// 1. Receives a raw IPC message into a stack buffer.
/// 2. Decodes it as `H::Request` (borrowing from the buffer).
/// 3. Calls `handler.handle()` — the request may borrow from the buffer.
/// 4. Encodes the response and sends it back to the original sender.
///
/// If `heartbeat_ticks > 0`, a receive timeout is used and
/// [`MessageHandler::handle`] is called with a synthesized heartbeat (not yet
/// implemented — pass `0` to use an infinite-wait recv).
///
/// # Panics
/// Panics if `handle` panics — callers should handle all errors internally and
/// return an error variant from `Response`.
pub fn run_service<H: MessageHandler>(mut handler: H, _heartbeat_ticks: u64) -> ! {
    let mut buf = [0u8; IPC_BUF_SIZE];
    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    loop {
        let sender_tid = match syscall::sys_recv(0, &mut buf) {
            syscall::SyscallResult::Ok(tid) if tid > 0 => tid,
            _ => {
                syscall::sys_yield();
                continue;
            }
        };

        // Decode request — borrows from `buf`, valid for the lifetime of this block.
        let req = match api::ipc::decode::<H::Request<'_>>(&buf) {
            Ok(r) => r,
            Err(_) => {
                // Malformed message — skip silently.  Don't crash the service.
                continue;
            }
        };

        let response = handler.handle(req, sender_tid);

        // Encode and reply.  If encoding fails, the response is a best-effort skip;
        // the caller will time out waiting but the service stays alive.
        match api::ipc::encode(&response, &mut resp_buf) {
            Ok(encoded) => {
                if let syscall::SyscallResult::Err(_) = syscall::sys_send(sender_tid, encoded) {
                    // Caller died before we could reply — not fatal, keep running.
                    let _ = ViError::IO; // suppress unused_variables
                }
            }
            Err(_) => {
                // Response too large to encode into IPC_BUF_SIZE.
                // This is a programming error in the handler — log and continue.
                let _ = syscall::sys_log("dispatch: response encode overflow");
            }
        }
    }
}
