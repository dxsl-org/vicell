// SPDX-License-Identifier: MPL-2.0
//! POSIX shim layer — Tier 1b C library support for ViCell cells.
//!
//! Provides `malloc`, string ops, file I/O, network sockets, entropy, math,
//! stdio, and setjmp as `#[no_mangle]` C-ABI symbols.  No picolibc or
//! toolchain libm required — math is backed by the Rust `libm` crate.
//!
//! Activates automatically on supported arches; cells must NOT link `-lm`.

#![allow(unsafe_code)]
#![allow(unused_variables)]
#![allow(non_upper_case_globals)]
#![cfg(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "wasm32", doc))]

// When the `mlibc` feature is active, all C symbols are provided by mlibc's
// libc.a (via the mlibc-shim crate).  Emitting them here too would cause
// duplicate-symbol link errors.  Gate every submodule and re-export off.
#[cfg(not(feature = "mlibc"))]
pub mod alloc;
#[cfg(not(feature = "mlibc"))]
pub mod entropy;
#[cfg(not(feature = "mlibc"))]
pub mod math;
#[cfg(not(feature = "mlibc"))]
pub mod net;
#[cfg(not(feature = "mlibc"))]
pub mod setjmp;
#[cfg(not(feature = "mlibc"))]
pub mod stdio_fmt;
#[cfg(not(feature = "mlibc"))]
pub mod stdio;
#[cfg(not(feature = "mlibc"))]
pub mod strings;
#[cfg(not(feature = "mlibc"))]
pub mod sysio;
#[cfg(not(feature = "mlibc"))]
pub mod cxxabi;

// Re-export the public API that was previously in the monolithic posix.rs,
// so existing callers (api::posix::getentropy, api::posix::socket, etc.) still compile.
#[cfg(not(feature = "mlibc"))]
pub use entropy::{getentropy, arc4random_buf};
#[cfg(not(feature = "mlibc"))]
pub use net::{sockaddr_in, socket, connect, send, recv, _close};
#[cfg(not(feature = "mlibc"))]
pub use stdio::FILE;
#[cfg(not(feature = "mlibc"))]
pub use sysio::_putchar;
