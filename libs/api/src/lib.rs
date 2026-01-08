// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Public API for ViOS.

#![no_std]

extern crate alloc;

// Export types so they are available via api::* if needed,
// and to satisfy `use crate::*` in modules if they rely on it.
pub use types::*;

pub mod allocator;
pub mod benchmark;
pub mod block;
pub mod driver;
pub mod fs;
pub mod hotswap;
pub mod net;
pub mod serde_helpers;
pub mod syscall;
pub mod vm;
pub mod async_io;
pub mod config;

pub use syscall::ViSyscall;
