#![no_std]

//! VFS Manager Service Cell - INTERFACE & IMPLEMENTATION (RAMFS)

extern crate alloc;
extern crate driver_disk; // Explicit extern crate

use ostd::prelude::*;
use api::fs::*;
use api::block::ViBlockDevice;
use types::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use driver_disk::RamDisk;

// Embed Binaries
static SHELL_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/shell");
static HELLO_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/hello");
static ECHO_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/echo");
static CAT_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/cat");
static LS_ELF: &[u8] = include_bytes!("../../../../kernel/src/embedded/ls");

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
    disk: RamDisk, // The raw block device
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
        let mut bin = Box::new(RamFile::new_dir("bin"));

        // Add embedded apps
        bin.children.insert(
            String::from("shell"),
            Box::new(RamFile::new_file("shell", SHELL_ELF))
        );
        bin.children.insert(
            String::from("hello"),
            Box::new(RamFile::new_file("hello", HELLO_ELF))
        );
        bin.children.insert(
            String::from("echo"),
            Box::new(RamFile::new_file("echo", ECHO_ELF))
        );
        bin.children.insert(
            String::from("cat"),
            Box::new(RamFile::new_file("cat", CAT_ELF))
        );
        bin.children.insert(
            String::from("ls"),
            Box::new(RamFile::new_file("ls", LS_ELF))
        );

        root.children.insert(String::from("bin"), bin);

        Self {
            root,
            disk: RamDisk::new(),
        }
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
}

// Implement ViFileSystem
impl ViFileSystem for VfsManager {
    fn open(&self, path: &str, _mode: OpenMode) -> ViResult<Box<dyn ViFile + Send + Sync>> {
        // Special handling for raw disk access
        if path == "/disk.img" {
            // Expose the raw disk as a file
            return Ok(Box::new(DiskFileHandle {
                // We use unsafe pointer to avoid lifetime issues for this prototype
                disk: &self.disk as *const RamDisk,
                pos: 0,
            }));
        }

        // Find the node
        if let Some(node) = self.find_node(path) {
            if node.is_dir {
                 // Open directory
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
        Err(ViError::NotSupported)
    }

    fn remove(&self, _path: &str) -> ViResult<()> {
        Err(ViError::NotSupported)
    }
}

struct DiskFileHandle {
    disk: *const RamDisk, // Raw pointer
    pos: u64,
}

unsafe impl Send for DiskFileHandle {}
unsafe impl Sync for DiskFileHandle {}

impl ViFile for DiskFileHandle {
    fn read(&mut self, buf: &mut [u8]) -> ViResult<usize> {
        // Read sector logic mock
        // We use unsafe to deref the disk pointer
        // SAFETY: VfsManager is assumed to be alive (static service)
        let disk = unsafe { &*self.disk };

        // Simple logic: read sector 0
        // To be real: use self.pos to find sector
        // Sector size = 512
        let sector = self.pos / 512;
        let offset = (self.pos % 512) as usize;

        let mut temp_buf = [0u8; 512];
        if disk.read_sector(sector, &mut temp_buf).is_ok() {
            let available = 512 - offset;
            let to_copy = core::cmp::min(buf.len(), available);
            buf[..to_copy].copy_from_slice(&temp_buf[offset..offset+to_copy]);
            self.pos += to_copy as u64;
            return Ok(to_copy);
        }

        Ok(0)
    }

    fn write(&mut self, _buf: &[u8]) -> ViResult<usize> { Err(ViError::NotSupported) }
    fn seek(&mut self, pos: SeekFrom) -> ViResult<u64> {
        match pos {
            SeekFrom::Start(p) => self.pos = p,
            _ => {},
        }
        Ok(self.pos)
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
