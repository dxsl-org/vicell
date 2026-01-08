// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Network API traits.

use crate::*;
use alloc::boxed::Box;

/// TCP/IP stack interface.
pub trait ViTcpStack: Send + Sync {
    /// Connect to a remote endpoint.
    fn connect(&self, addr: IpEndpoint) -> ViResult<Box<dyn ViTcpStream>>;
    
    /// Listen on a port.
    fn listen(&self, port: u16) -> ViResult<Box<dyn ViTcpListener>>;
}

/// TCP stream interface.
pub trait ViTcpStream: Send + Sync {
    /// Read data from stream.
    fn read(&mut self, buf: &mut [u8]) -> ViResult<usize>;
    
    /// Write data to stream.
    fn write(&mut self, buf: &[u8]) -> ViResult<usize>;
    
    /// Close the stream.
    fn close(&mut self) -> ViResult<()>;
}

/// TCP listener interface.
pub trait ViTcpListener: Send + Sync {
    /// Accept an incoming connection.
    fn accept(&self) -> ViResult<Box<dyn ViTcpStream>>;
}

/// IP endpoint (address + port).
#[derive(Debug, Clone, Copy)]
pub struct IpEndpoint {
    pub addr: IpAddr,
    pub port: u16,
}

/// IP address (v4 or v6).
#[derive(Debug, Clone, Copy)]
pub enum IpAddr {
    V4([u8; 4]),
    V6([u8; 16]),
}
