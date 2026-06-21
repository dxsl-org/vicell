//! Shell alias table — maps short names to replacement strings.
//!
//! Usage: `alias ll='ls -l'`  then typing `ll /bin` expands to `ls -l /bin`.
//! Aliases are session-local (persistence to `~/.ViCellrc` deferred to FAT32).

extern crate alloc;
use alloc::{collections::BTreeMap, string::String};

/// In-memory alias registry.
#[derive(Default)]
pub struct Aliases {
    map: BTreeMap<String, String>,
}

impl Aliases {
    pub fn new() -> Self { Self { map: BTreeMap::new() } }

    /// Define or update an alias.
    pub fn set(&mut self, name: &str, replacement: &str) {
        self.map.insert(String::from(name), String::from(replacement));
    }

    /// Remove an alias.
    pub fn remove(&mut self, name: &str) -> bool {
        self.map.remove(name).is_some()
    }

    /// Expand the first word of `line` if it matches an alias.
    ///
    /// Returns `Some(expanded)` if an alias was applied; `None` if the line
    /// should be passed through unchanged.
    pub fn expand(&self, line: &str) -> Option<String> {
        let first = line.split_whitespace().next()?;
        let replacement = self.map.get(first)?;
        let rest = line[first.len()..].trim_start();
        let mut expanded = String::from(replacement.as_str());
        if !rest.is_empty() {
            expanded.push(' ');
            expanded.push_str(rest);
        }
        Some(expanded)
    }

    /// Iterate all aliases as (name, value) pairs (for `alias` built-in display).
    pub fn list(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}
