// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Async I/O traits for non-blocking operations.
//!
//! These traits provide async variants of blocking I/O operations,
//! enabling high-performance concurrent I/O for network servers and
//! other latency-sensitive applications.

use crate::*;
use core::future::Future;
use core::pin::Pin;
use types::ViResult; // Fix import
use alloc::boxed::Box; // Fix import

/// Type alias for boxed futures.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Async filesystem interface.
///
/// Provides non-blocking file operations for concurrent I/O.
pub trait ViAsyncFileSystem: Send + Sync {
    /// Open a file asynchronously.
    fn open_async(&mut self, path: &str, mode: OpenMode) -> BoxFuture<'_, ViResult<Box<dyn ViAsyncFile>>>;
    
    /// Create a directory asynchronously.
    fn mkdir_async(&mut self, path: &str) -> BoxFuture<'_, ViResult<()>>;
    
    /// Remove a file or directory asynchronously.
    fn remove_async(&mut self, path: &str) -> BoxFuture<'_, ViResult<()>>;
}

/// Async file interface.
///
/// Provides non-blocking read/write operations.
pub trait ViAsyncFile: Send + Sync {
    /// Read data asynchronously.
    fn read_async<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, ViResult<usize>>;
    
    /// Write data asynchronously.
    fn write_async<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, ViResult<usize>>;
    
    /// Seek asynchronously.
    fn seek_async(&mut self, pos: SeekFrom) -> BoxFuture<'_, ViResult<u64>>;
}

/// Async TCP stack interface.
pub trait ViAsyncTcpStack: Send + Sync {
    /// Connect asynchronously.
    fn connect_async(&self, addr: IpEndpoint) -> BoxFuture<'_, ViResult<Box<dyn ViAsyncTcpStream>>>;
    
    /// Listen asynchronously.
    fn listen_async(&self, port: u16) -> BoxFuture<'_, ViResult<Box<dyn ViAsyncTcpListener>>>;
}

/// Async TCP stream interface.
pub trait ViAsyncTcpStream: Send + Sync {
    /// Read asynchronously.
    fn read_async<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, ViResult<usize>>;
    
    /// Write asynchronously.
    fn write_async<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, ViResult<usize>>;
    
    /// Close asynchronously.
    fn close_async(&mut self) -> BoxFuture<'_, ViResult<()>>;
}

/// Async TCP listener interface.
pub trait ViAsyncTcpListener: Send + Sync {
    /// Accept asynchronously.
    fn accept_async(&self) -> BoxFuture<'_, ViResult<Box<dyn ViAsyncTcpStream>>>;
}

/// Async block device interface.
pub trait ViAsyncBlockDevice: Send + Sync {
    /// Read sector asynchronously.
    fn read_sector_async<'a>(&'a self, sector: u64, buf: &'a mut [u8]) -> BoxFuture<'a, ViResult<()>>;
    
    /// Write sector asynchronously.
    fn write_sector_async<'a>(&'a self, sector: u64, buf: &'a [u8]) -> BoxFuture<'a, ViResult<()>>;
    
    /// Flush asynchronously.
    fn flush_async(&self) -> BoxFuture<'_, ViResult<()>>;
    
    /// Get sector count (synchronous, metadata only).
    fn sector_count(&self) -> u64;
    
    /// Get sector size (synchronous, metadata only).
    fn sector_size(&self) -> usize;
}

// Re-export from fs module for convenience
pub use crate::fs::{OpenMode, SeekFrom};
pub use crate::net::IpEndpoint;
