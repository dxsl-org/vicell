pub mod cursor;

use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal as VirtIOHal;
use core::ptr::NonNull;
// use log::info;
use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

pub struct GpuContext {
    pub gpu: VirtIOGpu<VirtIOHal, MmioTransport>,
    fb_ptr: *mut u8,
    fb_len: usize,
    pub width: u32,
    pub height: u32,
}

unsafe impl Send for GpuContext {}

pub static GPU_CONTEXT: Spinlock<Option<GpuContext>> = Spinlock::new(None);

/// Resource ID used by the framebuffer resource inside the vendored virtio-drivers.
/// Mirrors `RESOURCE_ID_FB` in kernel/third_party/virtio_drivers/src/device/gpu.rs.
/// Must stay in sync with that constant on any upstream bump.
const RESOURCE_ID_FB: u32 = 0xbabe;

impl GpuContext {
    pub fn framebuffer(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.fb_ptr, self.fb_len) }
    }

    /// Flush only the dirty sub-rectangle to the GPU, reducing DMA cost.
    ///
    /// Falls back to a full flush on error so the display is never left stale.
    pub fn flush_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        // Bounds-check: clamp to framebuffer dimensions to prevent out-of-range transfers.
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        if w == 0 || h == 0 { return; }
        // offset = byte position of top-left corner in the backing store.
        let offset = (y as u64 * self.width as u64 + x as u64) * 4;
        if self.gpu.flush_rect(x, y, w, h, offset).is_err() {
            // Fallback: full flush so the display is never left stale.
            let _ = self.gpu.flush();
        }
    }
}

pub fn init_driver() {
    log::info!("VirtIO GPU: Probing...");

    // We scan standard VirtIO MMIO slots (0x10001000 region)
    let transport_interval = 0x1000;

    for i in 0..8 {
        let addr = 0x1000_1000 + i * transport_interval;
        let header = unsafe { NonNull::new_unchecked((addr) as *mut VirtIOHeader) };

        match unsafe { MmioTransport::new(header) } {
            Ok(transport) => {
                let dev_type = transport.device_type();
                if dev_type != DeviceType::GPU {
                    // Dropping MmioTransport resets the device via set_status(0).
                    // For slots already owned by another driver (e.g. VirtIO block at slot 0),
                    // that reset would corrupt the driver's state.  `forget` prevents the Drop.
                    // SAFETY: the transport is valid but we intentionally skip cleanup to
                    // avoid resetting a device that belongs to another kernel driver.
                    core::mem::forget(transport);
                    continue;
                }
                if true /* dev_type == DeviceType::GPU */ {
                    log::info!("VirtIO GPU: Found at 0x{:X}", addr);
                    match VirtIOGpu::<VirtIOHal, MmioTransport>::new(transport) {
                        Ok(mut gpu) => {
                            // Probe resolution
                            let (width, height) = match gpu.resolution() {
                                Ok(res) => res,
                                Err(_) => (1280, 800), // Fallback
                            };
                            log::info!("VirtIO GPU: Probed Resolution: {}x{}", width, height);

                            // Setup 2D Resource
                            match gpu.setup_framebuffer() {
                                Ok(fb_slice) => {
                                    log::info!(
                                        "VirtIO GPU: Framebuffer setup success. Len: {}",
                                        fb_slice.len()
                                    );

                                    let fb_ptr = fb_slice.as_mut_ptr();
                                    let fb_len = fb_slice.len();

                                    *GPU_CONTEXT.lock() = Some(GpuContext {
                                        gpu,
                                        fb_ptr,
                                        fb_len,
                                        width,
                                        height,
                                    });

                                    // Flush
                                    if let Some(ctx) = GPU_CONTEXT.lock().as_mut() {
                                        let _ = ctx.gpu.flush();
                                    }

                                    // Init Framebuffer Console here
                                    crate::task::drivers::fb_console::FramebufferConsole::init();
                                }
                                Err(e) => {
                                    log::error!("VirtIO GPU: Setup Framebuffer failed: {:?}", e)
                                }
                            }
                            return;
                        }
                        Err(e) => {
                            log::error!("VirtIO GPU: Init failed: {:?}", e);
                        }
                    }
                }
            }
            Err(_) => {}
        }
    }
    log::warn!("VirtIO GPU: No device found.");
}
