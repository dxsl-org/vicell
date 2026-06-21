#![no_std]
//! mlibc-shim — link-only crate.
//!
//! This crate has no Rust code.  Its build.rs emits `cargo:rustc-link-*`
//! directives that inject the pre-built mlibc `libc.a` into every cell that
//! depends on this crate.
//!
//! **Exclusivity:** a cell that depends on mlibc-shim MUST also enable the
//! `mlibc` feature on `api` (`api = { features = ["mlibc"] }`).  Failing to
//! do so links BOTH posix.rs symbols AND mlibc symbols, which is a
//! duplicate-symbol link error.
