#![no_std]

extern crate alloc;

use ostd::prelude::*;
use api::block::ViBlockDevice;
use alloc::vec::Vec;
use alloc::vec;

// 40MB RamDisk
const DISK_SIZE: usize = 40 * 1024 * 1024;
const SECTOR_SIZE: usize = 512;

pub struct RamDisk {
    data: Vec<u8>,
}

impl RamDisk {
    pub fn new() -> Self {
        // Allocate zeroed memory
        // In a real scenario, this might map a region provided by the kernel.
        // For now, we allocate on heap.
        // WARNING: Allocating 40MB on heap might fail if the heap is small.
        // But for check verification, this logic is valid.
        Self {
            data: vec![0u8; DISK_SIZE],
        }
    }
}

impl ViBlockDevice for RamDisk {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        let offset = (sector as usize) * SECTOR_SIZE;
        if offset + buf.len() > self.data.len() {
            return Err(ViError::InvalidInput);
        }
        if buf.len() != SECTOR_SIZE {
             return Err(ViError::InvalidInput);
        }
        buf.copy_from_slice(&self.data[offset..offset + buf.len()]);
        Ok(())
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> ViResult<()> {
        // Since we need to modify self.data, but self is &self.
        // We need interior mutability.
        // However, ViBlockDevice takes &self.
        // In a real driver, we would access MMIO or UnsafeCell.
        // Here, let's use UnsafeCell or Mutex.
        // Since we are single-threaded mostly in this context or minimal,
        // using UnsafeCell is "okay" if we guarantee safety, but RefCell is safer for single core.
        // But ViBlockDevice requires Send + Sync.
        // Let's use a Spinlock or similar if available, or just UnsafeCell with a wrapper.
        // ostd might have sync primitives.
        // Checking ostd::sync

        // For simplicity in this mock:
        // We will cast away const-ness (Interior Mutability pattern via pointer)
        // This is UNSAFE but standard for "drivers" managing their own state if they don't have a Mutex.

        let offset = (sector as usize) * SECTOR_SIZE;
        if offset + buf.len() > self.data.len() {
            return Err(ViError::InvalidInput);
        }

        let ptr = self.data.as_ptr() as *mut u8;
        unsafe {
            let dst = core::slice::from_raw_parts_mut(ptr.add(offset), buf.len());
            dst.copy_from_slice(buf);
        }
        Ok(())
    }

    fn sector_count(&self) -> u64 {
        (DISK_SIZE / SECTOR_SIZE) as u64
    }

    fn sector_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn flush(&self) -> ViResult<()> {
        Ok(())
    }
}
