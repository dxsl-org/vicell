#![no_std]

//! VFS Manager Service Cell - INTERFACE & IMPLEMENTATION (RAMFS)

extern crate alloc;

use ostd::prelude::*;
use api::fs::*;
use types::*; // Import DirEntry, FileType directly from types crate
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

// Simple RamFS File Representation
#[derive(Clone)]
struct RamFile {
    name: String,
    data: Vec<u8>,
    is_dir: bool,
    // Simple children storage for directories
    children: BTreeMap<String, Box<RamFile>>,
}

impl RamFile {
    fn new_file(name: &str, data: &[u8]) -> Self {
        Self {
            name: String::from(name),
            data: Vec::from(data),
            is_dir: false,
            children: BTreeMap::new(),
        }
    }

    fn new_dir(name: &str) -> Self {
        Self {
            name: String::from(name),
            data: Vec::new(),
            is_dir: true,
            children: BTreeMap::new(),
        }
    }
}

pub struct VfsManager {
    root: Box<RamFile>,
}

impl VfsManager {
    pub fn new() -> Self {
        let mut root = Box::new(RamFile::new_dir("/"));

        // Add readme.txt
        root.children.insert(
            String::from("readme.txt"),
            Box::new(RamFile::new_file("readme.txt", b"Welcome to ViOS!\nThis is a file in RamFS.\n"))
        );

        // Add bin directory
        let bin = Box::new(RamFile::new_dir("bin"));
        // Maybe add some fake binaries?
        root.children.insert(String::from("bin"), bin);

        Self { root }
    }

    fn find_node(&self, path: &str) -> Option<&RamFile> {
        if path == "/" {
            return Some(&self.root);
        }

        let mut current = &self.root;
        for component in path.split('/').filter(|c| !c.is_empty()) {
            if let Some(next) = current.children.get(component) {
                current = next;
            } else {
                return None;
            }
        }
        Some(current)
    }

    pub fn mount(&mut self, _path: &str, _fs: Box<dyn ViFileSystem>) -> ViResult<()> {
        // Todo: Implement mount points
        Err(ViError::NotSupported)
    }

    pub fn unmount(&mut self, _path: &str) -> ViResult<()> {
        Err(ViError::NotSupported)
    }
}

// Implement ViFileSystem
impl ViFileSystem for VfsManager {
    fn open(&self, path: &str, _mode: OpenMode) -> ViResult<Box<dyn ViFile + Send + Sync>> {
        // Find the node
        if let Some(node) = self.find_node(path) {
            if node.is_dir {
                 // Open directory
                 // We need to collect entries
                 let entries: Vec<DirEntry> = node.children.values().map(|child| {
                     let mut entry = DirEntry::default();
                     let name_bytes = child.name.as_bytes();
                     let len = core::cmp::min(name_bytes.len(), 63);
                     entry.name[..len].copy_from_slice(&name_bytes[..len]);
                     entry.file_type = if child.is_dir { FileType::Directory } else { FileType::File };
                     entry.size = child.data.len() as u64;
                     entry
                 }).collect();

                 Ok(Box::new(RamFileHandle {
                     content: Vec::new(),
                     is_dir: true,
                     entries,
                     pos: 0,
                 }))
            } else {
                // Open file
                Ok(Box::new(RamFileHandle {
                    content: node.data.clone(),
                    is_dir: false,
                    entries: Vec::new(),
                    pos: 0,
                }))
            }
        } else {
            Err(ViError::NotFound)
        }
    }

    fn mkdir(&self, _path: &str) -> ViResult<()> {
        Err(ViError::NotSupported) // Read-only for now
    }

    fn remove(&self, _path: &str) -> ViResult<()> {
        Err(ViError::NotSupported)
    }
}

// File Handle Implementation
struct RamFileHandle {
    content: Vec<u8>,
    is_dir: bool,
    entries: Vec<DirEntry>,
    pos: usize,
}

impl ViFile for RamFileHandle {
    fn read(&mut self, buf: &mut [u8]) -> ViResult<usize> {
        if self.is_dir {
            return Err(ViError::IsADirectory);
        }
        if self.pos >= self.content.len() {
            return Ok(0);
        }
        let len = core::cmp::min(buf.len(), self.content.len() - self.pos);
        buf[..len].copy_from_slice(&self.content[self.pos..self.pos + len]);
        self.pos += len;
        Ok(len)
    }

    fn write(&mut self, _buf: &[u8]) -> ViResult<usize> {
        Err(ViError::NotSupported) // Read-only
    }

    fn seek(&mut self, pos: SeekFrom) -> ViResult<u64> {
        let new_pos = match pos {
            SeekFrom::Start(off) => off as i64,
            SeekFrom::End(off) => self.content.len() as i64 + off,
            SeekFrom::Current(off) => self.pos as i64 + off,
        };

        if new_pos < 0 {
            return Err(ViError::InvalidInput);
        }
        self.pos = new_pos as usize;
        Ok(self.pos as u64)
    }

    fn is_dir(&self) -> bool {
        self.is_dir
    }

    fn read_dir(&mut self) -> ViResult<Option<DirEntry>> {
        if !self.is_dir {
            return Err(ViError::NotADirectory);
        }
        if self.pos < self.entries.len() {
            let entry = self.entries[self.pos];
            self.pos += 1;
            Ok(Some(entry))
        } else {
            Ok(None)
        }
    }
}
