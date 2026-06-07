// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

// ostd - ViCell Standard Library
//
// Replacement for Rust's std library for ViCell Cells.
// INTERFACE ONLY - NO IMPLEMENTATION YET.

pub use api::*;

// Re-export alloc types
pub use alloc::boxed;
pub use alloc::string;
pub use alloc::vec;

/// Result type used throughout ViCell.
pub type Result<T, E = ViError> = core::result::Result<T, E>;

pub mod fast_ipc;
pub mod mmio;
pub mod startup;
pub mod sync;
pub mod syscall;

/// Allocator hooks (to be implemented).
pub mod heap;

/// I/O traits and functions.
pub mod io;

/// Filesystem.
pub mod fs;

/// Shared readline / REPL state machine (used by Shell).
pub mod repl;

pub mod prelude;

/// Executor
pub mod executor;

/// TLS 1.3 client helpers for app cells.
pub mod tls;

/// Task spawning.
pub mod task {
    use crate::*;

    /// Yield current task.
    pub fn yield_now() {
        syscall::sys_yield();
    }
}
