//! In-memory RamFS backend: read-only root catalog plus the volatile
//! read-write `/tmp` scratch space.
//!
//! Write/append are structurally restricted to `/tmp/` — the root catalog is
//! immutable by design. mkdir/rmdir/unlink operate anywhere in the tree;
//! the dispatcher's AccessTable is the authorization layer above this.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::backend::FsBackend;

#[derive(Clone)]
struct RamFile {
    data: Vec<u8>,
    is_dir: bool,
    children: BTreeMap<String, Box<RamFile>>,
}

impl RamFile {
    fn new_file(data: &[u8]) -> Self {
        Self { data: Vec::from(data), is_dir: false, children: BTreeMap::new() }
    }
    fn new_dir() -> Self {
        Self { data: Vec::new(), is_dir: true, children: BTreeMap::new() }
    }
}

pub struct RamFsBackend {
    root: Box<RamFile>,
}

impl RamFsBackend {
    pub fn new() -> Self {
        let mut root = Box::new(RamFile::new_dir());
        root.children.insert(String::from("readme.txt"),
            Box::new(RamFile::new_file(b"Welcome to ViCell!\n")));
        // /bin is served by BootFsProxy (kernel initramfs) since Phase 02 —
        // the embedded ELF copies that used to live here doubled every /bin
        // binary in RAM (once in kernel_fs.img, once in the VFS cell image).
        root.children.insert(String::from("tmp"), Box::new(RamFile::new_dir()));

        Self { root }
    }

    fn find_node(&self, path: &str) -> Option<&RamFile> {
        if path == "/" { return Some(&self.root); }
        let mut cur = &self.root;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = cur.children.get(part)?;
        }
        Some(cur)
    }

    fn find_node_mut(&mut self, path: &str) -> Option<&mut RamFile> {
        if path == "/" { return Some(&mut self.root); }
        let mut cur: &mut RamFile = &mut self.root;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = cur.children.get_mut(part)?.as_mut();
        }
        Some(cur)
    }

    /// Split `path` into (parent_path, child_name). Returns `None` for root "/".
    fn split_parent_name(path: &str) -> Option<(String, String)> {
        let path = path.trim_end_matches('/');
        if path.is_empty() { return None; }
        let slash = path.rfind('/')?;
        let parent = if slash == 0 { String::from("/") } else { String::from(&path[..slash]) };
        let name   = String::from(&path[slash + 1..]);
        if name.is_empty() { return None; }
        Some((parent, name))
    }

    fn get_file_data(&self, path: &str) -> Option<&[u8]> {
        let n = self.find_node(path)?;
        if n.is_dir { return None; }
        Some(&n.data)
    }

    /// Create or overwrite a regular file. Returns false if the parent does
    /// not exist or `path` names an existing directory.
    fn write_node(&mut self, path: &str, content: &[u8]) -> bool {
        let (parent_path, name) = match Self::split_parent_name(path) {
            Some(pn) => pn,
            None => return false,
        };
        let parent = match self.find_node_mut(&parent_path) {
            Some(p) if p.is_dir => p,
            _ => return false,
        };
        match parent.children.get_mut(&name) {
            Some(existing) if existing.is_dir => false,
            Some(existing) => { existing.data = Vec::from(content); true }
            None => {
                parent.children.insert(name, Box::new(RamFile::new_file(content)));
                true
            }
        }
    }
}

impl FsBackend for RamFsBackend {
    fn get_file_ptr(&self, path: &str) -> Option<(usize, usize)> {
        let n = self.find_node(path)?;
        if n.is_dir { return None; }
        Some((n.data.as_ptr() as usize, n.data.len()))
    }

    fn list(&self, path: &str, out: &mut [u8]) -> usize {
        let node = match self.find_node(path) {
            Some(n) if n.is_dir => n,
            _ => return 0,
        };
        // Historical 480-byte cap (pre-MountTable dispatch staged through a
        // 480-byte temp buffer) — kept so reply sizes stay bit-identical.
        let cap = out.len().min(480);
        let mut pos = 0;
        for (name, child) in node.children.iter() {
            let prefix: &[u8] = if child.is_dir { b"d:" } else { b"f:" };
            let name_b = name.as_bytes();
            let entry_len = prefix.len() + name_b.len() + 1; // +1 for '\n'
            if pos + entry_len > cap { break; }
            out[pos..pos + 2].copy_from_slice(prefix);
            out[pos + 2..pos + 2 + name_b.len()].copy_from_slice(name_b);
            out[pos + 2 + name_b.len()] = b'\n';
            pos += entry_len;
        }
        pos
    }

    fn stat(&self, path: &str) -> Option<(u64, bool)> {
        self.find_node(path).map(|n| (n.data.len() as u64, n.is_dir))
    }

    fn file_size(&self, path: &str) -> u64 {
        self.get_file_data(path).map(|d| d.len() as u64).unwrap_or(0)
    }

    fn read_to_vec(&self, path: &str) -> Vec<u8> {
        self.get_file_data(path).map(|d| d.to_vec()).unwrap_or_default()
    }

    fn write(&mut self, path: &str, content: &[u8]) -> bool {
        // Embedded catalog is immutable; only the volatile scratch space accepts writes.
        if !path.starts_with("/tmp/") { return false; }
        self.write_node(path, content)
    }

    fn append(&mut self, path: &str, content: &[u8]) -> bool {
        if !path.starts_with("/tmp/") { return false; }
        let mut data = self.get_file_data(path).map(|d| d.to_vec()).unwrap_or_default();
        data.extend_from_slice(content);
        self.write_node(path, &data)
    }

    fn mkdir(&mut self, path: &str) -> bool {
        if let Some((parent_path, name)) = Self::split_parent_name(path) {
            if let Some(parent) = self.find_node_mut(&parent_path) {
                if parent.is_dir && !parent.children.contains_key(&name) {
                    parent.children.insert(name, Box::new(RamFile::new_dir()));
                    return true;
                }
            }
        }
        false
    }

    fn rmdir(&mut self, path: &str) -> bool {
        if let Some((parent_path, name)) = Self::split_parent_name(path) {
            if let Some(parent) = self.find_node_mut(&parent_path) {
                let removable = parent.children.get(&name)
                    .map(|c| c.is_dir && c.children.is_empty())
                    .unwrap_or(false);
                if removable {
                    parent.children.remove(&name);
                    return true;
                }
            }
        }
        false
    }

    fn unlink(&mut self, path: &str) -> bool {
        if let Some((parent_path, name)) = Self::split_parent_name(path) {
            if let Some(parent) = self.find_node_mut(&parent_path) {
                let removable = parent.children.get(&name)
                    .map(|c| !c.is_dir)
                    .unwrap_or(false);
                if removable {
                    parent.children.remove(&name);
                    return true;
                }
            }
        }
        false
    }

    fn rmdir_recursive(&mut self, _path: &str) -> bool {
        false // recursive delete is only supported on the persistent volume
    }
}
