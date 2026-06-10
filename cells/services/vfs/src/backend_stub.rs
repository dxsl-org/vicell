//! Stub backend — placeholder for `/srv` until the G2 native filesystem
//! (RedoxFS port) is implemented alongside NVMe support.
//! See `docs/specs/09b-vfs-native-fs-adr.md` for the architectural decision.

use alloc::vec::Vec;

use crate::backend::FsBackend;

pub struct StubBackend;

impl StubBackend {
    pub fn new() -> Self {
        Self
    }
}

impl FsBackend for StubBackend {
    fn get_file_ptr(&self, _path: &str) -> Option<(usize, usize)> { None }
    fn list(&self, _path: &str, _out: &mut [u8]) -> usize { 0 }
    fn stat(&self, _path: &str) -> Option<(u64, bool)> { None }
    fn file_size(&self, _path: &str) -> u64 { 0 }
    fn read_to_vec(&self, _path: &str) -> Vec<u8> { Vec::new() }
    fn write(&mut self, _path: &str, _content: &[u8]) -> bool { false }
    fn append(&mut self, _path: &str, _content: &[u8]) -> bool { false }
    fn mkdir(&mut self, _path: &str) -> bool { false }
    fn rmdir(&mut self, _path: &str) -> bool { false }
    fn unlink(&mut self, _path: &str) -> bool { false }
    fn rmdir_recursive(&mut self, _path: &str) -> bool { false }
}
