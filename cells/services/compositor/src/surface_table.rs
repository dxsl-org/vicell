//! CapId-keyed surface state table.
//!
//! Each surface references pixel data via one of two sources:
//!   - `PixelSource::Grant` — a read-only pointer into the app cell's own Grant buffer
//!     (zero-copy; app writes directly, compositor reads directly).
//!   - `PixelSource::Owned` — a compositor-owned buffer (legacy `WRITE_PIXELS` path).
//!
//! New code should always use the Grant path via `attach_grant`.

extern crate alloc;

use alloc::{boxed::Box, collections::BTreeMap};
use api::display::{PixelFormat, Rect};
use types::ViError;

/// Maximum number of simultaneous surfaces.
///
/// 32 covers typical desktop use; reduce to 2 for kiosk/embedded profiles.
pub const MAX_SURFACES: usize = 32;

/// Pixel data source for a surface.
#[allow(dead_code)] // reg_id reserved for future cleanup on cell exit
enum PixelSource {
    /// App Cell's Grant buffer — compositor reads directly via a read-only pointer.
    ///
    /// SAFETY invariant: `ptr` is valid for reads as long as the owning cell holds its
    /// `sys_grant_register` buffer.  The protocol requires the app to send `DETACH_GRANT`
    /// before calling `sys_grant_unregister`, so the pointer is never dangling during a
    /// render tick.  The compositor never writes through this pointer.
    Grant { ptr: *const u8, reg_id: usize },
    /// Compositor-owned fallback buffer (legacy `WRITE_PIXELS` path).
    Owned(Box<[u8]>),
}

// SAFETY: compositor runs as a single cooperative task; no other task touches
// PixelSource concurrently.  The Grant pointer is a stable physical page whose
// lifetime is enforced by the DETACH_GRANT protocol above.
unsafe impl Send for PixelSource {}

/// State for one live surface.
pub struct SurfaceState {
    /// Screen position.
    pub x: i32,
    pub y: i32,
    /// Dimensions in pixels.
    pub w: u32,
    pub h: u32,
    /// Pixel format (default: Bgra8888).
    pub fmt: PixelFormat,
    /// Pixel data source.
    source: PixelSource,
    /// Accumulated damage since last flush.  `None` = no damage.
    pub damage: Option<Rect>,
    /// TID of the cell that created this surface (input routing + ownership checks).
    pub owner: usize,
}

impl SurfaceState {
    /// Allocate a new surface with a compositor-owned pixel buffer.
    ///
    /// Used by `CREATE_SURFACE` before the app attaches a Grant.  Preserves
    /// compatibility with the legacy `WRITE_PIXELS` path.
    pub fn new(x: i32, y: i32, w: u32, h: u32, owner: usize) -> Self {
        // No eager pixel buffer. The Grant path (attach_grant) is the standard flow
        // and replaces `source` immediately after CREATE_SURFACE, so pre-allocating
        // w*h*4 here is wasted — and for a full-screen surface (3 MiB at 1024×768) it
        // would briefly sit on top of the compositor's own framebuffers and OOM its
        // 8 MiB cell heap. The legacy WRITE_PIXELS path grows the buffer lazily.
        Self {
            x, y, w, h,
            fmt: PixelFormat::Bgra8888,
            source: PixelSource::Owned(alloc::vec::Vec::new().into_boxed_slice()),
            damage: None,
            owner,
        }
    }

    /// Attach a Grant buffer from the app cell.
    ///
    /// `ptr` comes from `sys_grant_slice` after the app shared the grant read-only.
    /// Updates dimensions and format from the `AttachGrant` message.
    pub fn attach_grant(&mut self, ptr: *const u8, reg_id: usize,
                        w: u32, h: u32, fmt: PixelFormat) {
        self.w = w;
        self.h = h;
        self.fmt = fmt;
        self.source = PixelSource::Grant { ptr, reg_id };
    }

    /// Detach the Grant and fall back to an empty Owned buffer.
    ///
    /// Must be called when the app sends `DETACH_GRANT`, before it frees the Grant.
    /// We do NOT eagerly allocate a full-screen replacement — a large surface would
    /// OOM the compositor heap, and a surface is almost always destroyed right after
    /// detach. The legacy WRITE_PIXELS path regrows the buffer lazily if reused.
    pub fn detach_grant(&mut self) {
        self.source = PixelSource::Owned(alloc::vec::Vec::new().into_boxed_slice());
    }

    /// Read access to pixel data — either from the Grant or the Owned buffer.
    pub fn pixels(&self) -> &[u8] {
        match &self.source {
            PixelSource::Grant { ptr, .. } => {
                let len = (self.w * self.h * self.fmt.bpp()) as usize;
                // SAFETY: ptr comes from sys_grant_slice after the app called
                // sys_grant_share(perm=0, ReadOnly).  The buffer is registered
                // (sys_grant_register) for the surface's lifetime; the app sends
                // DETACH_GRANT before sys_grant_unregister, ensuring the pointer
                // is valid for any render tick that follows ATTACH_GRANT.
                // The compositor only reads — no write through this pointer.
                unsafe { core::slice::from_raw_parts(*ptr, len) }
            }
            PixelSource::Owned(buf) => buf,
        }
    }

    /// Write pixel data into an Owned surface (legacy `WRITE_PIXELS` path).
    ///
    /// Silently ignores writes on Grant surfaces — those are written directly by
    /// the app cell.
    pub fn write_pixels(&mut self, px: i32, py: i32, pw: u32, ph: u32, data: &[u8]) {
        // Lazily allocate the legacy owned buffer to full surface size on first use
        // (new()/detach_grant() leave it empty to avoid eager full-screen allocs).
        let needed = (self.w * self.h * 4) as usize;
        if let PixelSource::Owned(b) = &self.source {
            if b.len() < needed {
                self.source = PixelSource::Owned(alloc::vec![0u8; needed].into_boxed_slice());
            }
        }
        let buf = match &mut self.source {
            PixelSource::Owned(b) => b,
            PixelSource::Grant { .. } => return,
        };
        let expected = (pw * ph * 4) as usize;
        if data.len() < expected { return; }
        let stride = self.w as usize * 4;
        for row in 0..ph as usize {
            let dst_off = (py as usize + row) * stride + px as usize * 4;
            let src_off = row * pw as usize * 4;
            let row_bytes = pw as usize * 4;
            if dst_off + row_bytes <= buf.len() {
                buf[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&data[src_off..src_off + row_bytes]);
            }
        }
        let new_dmg = Rect { x: px, y: py, w: pw, h: ph };
        self.damage = Some(match self.damage {
            Some(existing) => existing.union(&new_dmg),
            None => new_dmg,
        });
    }

    /// Clear the damage accumulator after a flush.
    pub fn clear_damage(&mut self) { self.damage = None; }

    /// Bounding rect of this surface on screen.
    pub fn screen_rect(&self) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w, h: self.h }
    }
}

/// CapId-keyed surface registry.
#[derive(Default)]
pub struct SurfaceTable {
    entries:  BTreeMap<u64, SurfaceState>,
    next_cap: u64,
}

impl SurfaceTable {
    pub fn new() -> Self { Self { entries: BTreeMap::new(), next_cap: 1 } }

    /// Allocate a new surface slot and return its CapId.
    ///
    /// `owner` is the TID of the creating cell (ownership checks + input routing).
    ///
    /// # Errors
    /// Returns `OutOfMemory` if `MAX_SURFACES` is already reached.
    pub fn create(&mut self, x: i32, y: i32, w: u32, h: u32, owner: usize)
        -> Result<u64, ViError>
    {
        if self.entries.len() >= MAX_SURFACES { return Err(ViError::OutOfMemory); }
        let cap = self.next_cap;
        self.next_cap += 1;
        self.entries.insert(cap, SurfaceState::new(x, y, w, h, owner));
        Ok(cap)
    }

    /// Look up a surface mutably.
    pub fn get_mut(&mut self, cap: u64) -> Option<&mut SurfaceState> {
        self.entries.get_mut(&cap)
    }

    /// Look up a surface immutably.
    pub fn get(&self, cap: u64) -> Option<&SurfaceState> {
        self.entries.get(&cap)
    }

    /// Remove a surface.
    pub fn remove(&mut self, cap: u64) -> Option<SurfaceState> {
        self.entries.remove(&cap)
    }

    /// Returns true if any live surface has accumulated damage.
    ///
    /// Used by the main loop to decide whether to call render_frame.
    pub fn has_damage(&self) -> bool {
        self.entries.values().any(|s| s.damage.is_some())
    }

    /// Find all surfaces owned by `tid` and return their caps.
    #[allow(dead_code)] // used by future NotifyOnExit cleanup path
    pub fn caps_owned_by(&self, tid: usize) -> alloc::vec::Vec<u64> {
        self.entries.iter()
            .filter(|(_, s)| s.owner == tid)
            .map(|(&cap, _)| cap)
            .collect()
    }
}
