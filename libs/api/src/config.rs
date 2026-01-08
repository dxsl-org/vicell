// SPDX-License-Identifier: MPL-2.0

//! Configuration API traits.

use crate::*;
use types::ViResult; // Fix import
use alloc::string::String;
use alloc::vec::Vec;

/// Configuration Service Interface.
pub trait ViConfig: Send + Sync {
    /// Get a configuration value.
    fn get(&self, key: &str) -> ViResult<String>;

    /// Set a configuration value.
    fn set(&self, key: &str, value: &str) -> ViResult<()>;
}
