//! Safe MMIO accessor for Driver Cells.
//!
//! Driver Cells use `#![forbid(unsafe_code)]` and cannot directly deref raw
//! hardware addresses.  `MmioRegion` hides the `unsafe` inside this trusted
//! library and enforces bounds checking on every access.
//!
//! A Cell cannot construct `MmioRegion` directly — the constructor is
//! `pub(crate)`.  Cells obtain one by calling `request_region()`, which
//! goes through the kernel Resource Registry syscall (Phase 02).

use types::{ViError, ViResult};
use crate::syscall;

/// Request exclusive access to the MMIO range `[base, base+len)` from the kernel.
///
/// On success, returns an `MmioRegion` the Driver Cell can use for volatile
/// register reads and writes.  The kernel:
/// 1. Checks `[base, base+len)` against the per-arch allowlist.
/// 2. Verifies no other Cell currently owns the range.
/// 3. Checks the caller's ELF manifest declared the matching peripheral cap.
///
/// # Errors
/// - `PermissionDenied` — range not in allowlist or manifest cap missing.
/// - `AlreadyExists` — another Cell owns an overlapping range.
/// - `InvalidInput` — arithmetic overflow in `base + len`.
pub fn request_region(base: usize, len: usize) -> ViResult<MmioRegion> {
    let ret = syscall::sys_request_mmio(base, len);
    if ret == 0 {
        // SAFETY: kernel approved the range; it is valid device MMIO.
        Ok(MmioRegion::new(base, len))
    } else {
        Err(match ret {
            1 => ViError::PermissionDenied,
            2 => ViError::AlreadyExists,
            _ => ViError::InvalidInput,
        })
    }
}

/// A bounds-checked, volatile read/write accessor for a device MMIO window.
///
/// Fields are private; Cell code (a separate Rust crate) cannot forge an
/// `MmioRegion`.  Obtain one via [`request_region`].
pub struct MmioRegion {
    base: usize,
    len: usize,
}

impl MmioRegion {
    /// Construct an `MmioRegion` for `[base, base+len)`.
    ///
    /// `pub(crate)` — only ostd internals (i.e. `request_region`) call this
    /// after the kernel has confirmed exclusive access for the calling Cell.
    pub(crate) fn new(base: usize, len: usize) -> Self {
        Self { base, len }
    }

    /// Read a `T`-sized value at `offset` bytes from the region base.
    ///
    /// Returns `Err(InvalidInput)` if `offset + size_of::<T>()` would exceed
    /// `self.len` or if the arithmetic overflows.
    pub fn read<T: Copy>(&self, offset: usize) -> ViResult<T> {
        let size = core::mem::size_of::<T>();
        let end = offset.checked_add(size).ok_or(ViError::InvalidInput)?;
        if end > self.len {
            return Err(ViError::InvalidInput);
        }
        let ptr = (self.base + offset) as *const T;
        // SAFETY: bounds-check above ensures [ptr, ptr+size) ⊆ [base, base+len).
        // The Region was granted by the kernel Resource Registry, guaranteeing
        // the address range is valid device MMIO.  `read_volatile` prevents the
        // compiler from caching or eliding the hardware read.
        Ok(unsafe { core::ptr::read_volatile(ptr) })
    }

    /// Write `val` of type `T` at `offset` bytes from the region base.
    ///
    /// Returns `Err(InvalidInput)` on out-of-bounds.
    pub fn write<T: Copy>(&self, offset: usize, val: T) -> ViResult<()> {
        let size = core::mem::size_of::<T>();
        let end = offset.checked_add(size).ok_or(ViError::InvalidInput)?;
        if end > self.len {
            return Err(ViError::InvalidInput);
        }
        let ptr = (self.base + offset) as *mut T;
        // SAFETY: same contract as `read` above.
        unsafe { core::ptr::write_volatile(ptr, val) };
        Ok(())
    }

    /// Read a 32-bit device register at `offset` (most common register width).
    #[inline]
    pub fn read_u32(&self, offset: usize) -> ViResult<u32> {
        self.read::<u32>(offset)
    }

    /// Write a 32-bit device register at `offset`.
    #[inline]
    pub fn write_u32(&self, offset: usize, val: u32) -> ViResult<()> {
        self.write::<u32>(offset, val)
    }

    /// Read an 8-bit device register at `offset`.
    #[inline]
    pub fn read_u8(&self, offset: usize) -> ViResult<u8> {
        self.read::<u8>(offset)
    }

    /// Write an 8-bit device register at `offset`.
    #[inline]
    pub fn write_u8(&self, offset: usize, val: u8) -> ViResult<()> {
        self.write::<u8>(offset, val)
    }

    /// Base address of this region (physical/MMIO address).
    #[inline]
    pub fn base(&self) -> usize {
        self.base
    }

    /// Length of this region in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the region has zero length.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: back an MmioRegion with a stack buffer for testing.
    fn make_region(buf: &mut [u8]) -> MmioRegion {
        MmioRegion::new(buf.as_mut_ptr() as usize, buf.len())
    }

    #[test]
    fn rejects_oob_read() {
        let mut buf = [0u8; 4];
        let r = make_region(&mut buf);
        // offset=0, size=4 → end=4 == len=4 → OK
        assert!(r.read_u32(0).is_ok());
        // offset=1, size=4 → end=5 > len=4 → Err
        assert_eq!(r.read_u32(1), Err(ViError::InvalidInput));
        // offset=4, size=4 → end=8 > len=4 → Err
        assert_eq!(r.read_u32(4), Err(ViError::InvalidInput));
    }

    #[test]
    fn rejects_oob_write() {
        let mut buf = [0u8; 4];
        let r = make_region(&mut buf);
        assert!(r.write_u32(0, 0x1234_5678).is_ok());
        assert_eq!(r.write_u32(1, 0), Err(ViError::InvalidInput));
    }

    #[test]
    fn round_trip_u32() {
        let mut buf = [0u8; 16];
        let r = make_region(&mut buf);
        r.write_u32(0, 0xDEAD_BEEF).unwrap();
        assert_eq!(r.read_u32(0).unwrap(), 0xDEAD_BEEF);
        r.write_u32(4, 0x1234_5678).unwrap();
        assert_eq!(r.read_u32(4).unwrap(), 0x1234_5678);
        // independent offsets do not clobber each other
        assert_eq!(r.read_u32(0).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn round_trip_u8() {
        let mut buf = [0u8; 4];
        let r = make_region(&mut buf);
        r.write_u8(0, 0xAB).unwrap();
        r.write_u8(3, 0xCD).unwrap();
        assert_eq!(r.read_u8(0).unwrap(), 0xAB);
        assert_eq!(r.read_u8(3).unwrap(), 0xCD);
    }

    #[test]
    fn overflow_offset_rejected() {
        let mut buf = [0u8; 4];
        let r = make_region(&mut buf);
        // offset near usize::MAX + size_of::<u32>()=4 must not overflow
        assert_eq!(r.read_u32(usize::MAX - 2), Err(ViError::InvalidInput));
        assert_eq!(r.write_u32(usize::MAX, 0), Err(ViError::InvalidInput));
    }
}
