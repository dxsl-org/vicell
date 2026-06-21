//! Hot-swap state transfer for the shell cell.
// Hotswap types are invoked by the kernel orchestrator at runtime, not from
// sibling Rust modules — dead-code lints are expected and intentional.
#![allow(dead_code)] // reason: all public items called by the HotSwap kernel orchestrator
//!
//! Serialises the command history and alias table so that a live shell upgrade
//! (`hotswap shell /bin/shell-v2`) preserves the user's session context.
//!
//! Wire format (little-endian, schema v1):
//! ```text
//! [schema_version: u32]
//! [history_count: u32]
//!   [entry_len: u16][entry bytes]...
//! [alias_count: u32]
//!   [name_len: u16][name bytes][val_len: u16][val bytes]...
//! ```

extern crate alloc;

use alloc::vec::Vec;
use api::hotswap::ViStateTransfer;
use ostd::prelude::*;
use crate::history::History;
use crate::aliases::Aliases;

const SCHEMA_VERSION: u32 = 1;

/// Serialisable snapshot of the shell's session state.
pub struct ShellState<'a> {
    pub history: &'a History,
    pub aliases: &'a Aliases,
}

impl<'a> ShellState<'a> {
    pub fn new(history: &'a History, aliases: &'a Aliases) -> Self {
        Self { history, aliases }
    }
}

impl<'a> ViStateTransfer for ShellState<'a> {
    fn state_size(&self) -> usize {
        let hist_bytes: usize = (0..self.history.len())
            .filter_map(|i| self.history.get(i))
            .map(|e| 2 + e.len())
            .sum();
        let alias_bytes: usize = self.aliases.list()
            .map(|(k, v)| 2 + k.len() + 2 + v.len())
            .sum();
        // version(4) + hist_count(4) + hist entries + alias_count(4) + alias entries
        4 + 4 + hist_bytes + 4 + alias_bytes
    }

    fn serialize_state(&self, buf: &mut [u8]) -> ViResult<usize> {
        let needed = self.state_size();
        if buf.len() < needed { return Err(ViError::InvalidArgument); }
        let mut pos = 0;

        // Header
        buf[pos..pos+4].copy_from_slice(&SCHEMA_VERSION.to_le_bytes()); pos += 4;

        // History
        let hist_count = self.history.len() as u32;
        buf[pos..pos+4].copy_from_slice(&hist_count.to_le_bytes()); pos += 4;
        for i in 0..self.history.len() {
            if let Some(entry) = self.history.get(i) {
                let el = entry.len() as u16;
                buf[pos..pos+2].copy_from_slice(&el.to_le_bytes()); pos += 2;
                buf[pos..pos+entry.len()].copy_from_slice(entry.as_bytes()); pos += entry.len();
            }
        }

        // Aliases
        let alias_vec: Vec<(&str, &str)> = self.aliases.list().collect();
        let alias_count = alias_vec.len() as u32;
        buf[pos..pos+4].copy_from_slice(&alias_count.to_le_bytes()); pos += 4;
        for (k, v) in &alias_vec {
            let kl = k.len() as u16;
            let vl = v.len() as u16;
            buf[pos..pos+2].copy_from_slice(&kl.to_le_bytes()); pos += 2;
            buf[pos..pos+k.len()].copy_from_slice(k.as_bytes()); pos += k.len();
            buf[pos..pos+2].copy_from_slice(&vl.to_le_bytes()); pos += 2;
            buf[pos..pos+v.len()].copy_from_slice(v.as_bytes()); pos += v.len();
        }

        Ok(pos)
    }

    fn deserialize_state(&mut self, _buf: &[u8]) -> ViResult<()> {
        // Deserialization requires a mutable ShellState — handled by the receiving
        // shell instance via `ShellStateOwned::deserialize_state` below.
        Err(ViError::NotSupported)
    }
}

/// Mutable shell state for deserialisation into a new shell instance.
pub struct ShellStateOwned {
    pub history: History,
    pub aliases: Aliases,
}

impl ShellStateOwned {
    pub fn empty() -> Self {
        Self { history: History::new(), aliases: Aliases::new() }
    }
}

impl ViStateTransfer for ShellStateOwned {
    fn state_size(&self) -> usize { 0 } // not used for deserialise-only path

    fn serialize_state(&self, _buf: &mut [u8]) -> ViResult<usize> {
        Err(ViError::NotSupported)
    }

    fn deserialize_state(&mut self, buf: &[u8]) -> ViResult<()> {
        if buf.len() < 8 { return Err(ViError::InvalidInput); }
        let _version = u32::from_le_bytes([buf[0],buf[1],buf[2],buf[3]]);
        let hist_count = u32::from_le_bytes([buf[4],buf[5],buf[6],buf[7]]) as usize;
        let mut pos = 8usize;

        for _ in 0..hist_count {
            if pos + 2 > buf.len() { return Err(ViError::InvalidInput); }
            let el = u16::from_le_bytes([buf[pos], buf[pos+1]]) as usize; pos += 2;
            if pos + el > buf.len() { return Err(ViError::InvalidInput); }
            let s = core::str::from_utf8(&buf[pos..pos+el]).map_err(|_| ViError::InvalidInput)?;
            self.history.push(s); pos += el;
        }

        if pos + 4 > buf.len() { return Ok(()); } // graceful — older version
        let alias_count = u32::from_le_bytes([buf[pos],buf[pos+1],buf[pos+2],buf[pos+3]]) as usize;
        pos += 4;
        for _ in 0..alias_count {
            if pos + 2 > buf.len() { return Err(ViError::InvalidInput); }
            let kl = u16::from_le_bytes([buf[pos], buf[pos+1]]) as usize; pos += 2;
            if pos + kl > buf.len() { return Err(ViError::InvalidInput); }
            let k = core::str::from_utf8(&buf[pos..pos+kl]).map_err(|_| ViError::InvalidInput)?;
            pos += kl;
            if pos + 2 > buf.len() { return Err(ViError::InvalidInput); }
            let vl = u16::from_le_bytes([buf[pos], buf[pos+1]]) as usize; pos += 2;
            if pos + vl > buf.len() { return Err(ViError::InvalidInput); }
            let v = core::str::from_utf8(&buf[pos..pos+vl]).map_err(|_| ViError::InvalidInput)?;
            pos += vl;
            self.aliases.set(k, v);
        }
        Ok(())
    }
}
