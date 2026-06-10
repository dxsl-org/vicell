// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Public API for ViCell.

// Disable `no_std` when running the test harness so `#[test]` can link
// against the host libstd.  All production builds remain bare-metal.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

// Export types so they are available via api::* if needed,
// and to satisfy `use crate::*` in modules if they rely on it.
pub use types::*;

pub mod allocator;
pub mod async_io;
pub mod fast_ipc;
pub mod ipc;
pub mod task;
pub use task::TaskPriority;
pub mod cap;
pub mod benchmark;
pub mod block;
pub mod config;
pub mod driver;
pub mod display;
pub mod fs;
pub mod disk;
pub mod hotswap;
pub mod input;
pub mod manifest;
pub mod net;
pub mod posix;
pub mod serde_helpers;
pub mod syscall;
pub mod vm;

// POSIX Shim Layer


pub use syscall::ViSyscall;

pub mod syscall_tests;
