//! Limine bootloader protocol structures and requests.
//!
//! This module defines the Limine protocol for communicating with the bootloader.
//! See: https://github.com/limine-bootloader/limine/blob/trunk/PROTOCOL.md

/// Limine protocol magic values
const LIMINE_COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

/// Limine base-revision tag (v8 protocol, revision 3).
/// Layout: [identifier_magic0, identifier_magic1, revision].
/// The bootloader writes back the revision it will honour.
#[used]
#[link_section = ".requests"]
static LIMINE_BASE_REVISION: [u64; 3] = [
    0xf9562b2d5c95a6c8,
    0x6a7b384944536bdc,
    3,
];

/// Limine request-section delimiter: start marker (required by rev 2+).
#[used]
#[link_section = ".requests_start_marker"]
static REQUESTS_START_MARKER: [u64; 2] = [0xf6b8f4b39de7716f, 0xfaa4f786d5a15bc4];

/// Limine request-section delimiter: end marker (required by rev 2+).
#[used]
#[link_section = ".requests_end_marker"]
static REQUESTS_END_MARKER: [u64; 2] = [0xadc0e0531bb10d03, 0x9572709f31764c62];

/// Memory map request
#[repr(C)]
pub struct LimineMemoryMapRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineMemoryMapResponse,
}

unsafe impl Send for LimineMemoryMapRequest {}
unsafe impl Sync for LimineMemoryMapRequest {}

#[repr(C)]
pub struct LimineMemoryMapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: *const *const LimineMemoryMapEntry,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LimineMemoryMapEntry {
    pub base: u64,
    pub length: u64,
    pub entry_type: u64,
}

#[used]
#[link_section = ".requests"]
static mut MEMORY_MAP_REQUEST: LimineMemoryMapRequest = LimineMemoryMapRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x67cf3d9d378a806f,
        0xe304acdfc50c3c62,
    ],
    revision: 0,
    response: core::ptr::null(),
};

/// Framebuffer request
#[repr(C)]
pub struct LimineFramebufferRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineFramebufferResponse,
}

unsafe impl Send for LimineFramebufferRequest {}
unsafe impl Sync for LimineFramebufferRequest {}

#[repr(C)]
pub struct LimineFramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *const *const LimineFramebuffer,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LimineFramebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *const u8,
}

#[used]
#[link_section = ".requests"]
static mut FRAMEBUFFER_REQUEST: LimineFramebufferRequest = LimineFramebufferRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x9d5827dcd881dd75,
        0xa3148604f6fab11b,
    ],
    revision: 0,
    response: core::ptr::null(),
};

/// HHDM (Higher Half Direct Map) request
#[repr(C)]
pub struct LimineHhdmRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineHhdmResponse,
}

unsafe impl Send for LimineHhdmRequest {}
unsafe impl Sync for LimineHhdmRequest {}

#[repr(C)]
pub struct LimineHhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[used]
#[link_section = ".requests"]
static mut HHDM_REQUEST: LimineHhdmRequest = LimineHhdmRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x48dcf1cb8ad2b852,
        0x63984e959a98244b,
    ],
    revision: 0,
    response: core::ptr::null(),
};

/// Kernel address request
#[repr(C)]
pub struct LimineKernelAddressRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineKernelAddressResponse,
}

unsafe impl Send for LimineKernelAddressRequest {}
unsafe impl Sync for LimineKernelAddressRequest {}

#[repr(C)]
pub struct LimineKernelAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

#[used]
#[link_section = ".requests"]
static mut KERNEL_ADDRESS_REQUEST: LimineKernelAddressRequest = LimineKernelAddressRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x71ba76863cc55f63,
        0xb2644a48c516a487,
    ],
    revision: 0,
    response: core::ptr::null(),
};

/// Get memory map from Limine
pub fn get_memory_map() -> Option<&'static LimineMemoryMapResponse> {
    unsafe {
        let response = MEMORY_MAP_REQUEST.response;
        if response.is_null() {
            None
        } else {
            Some(&*response)
        }
    }
}

/// Get framebuffer from Limine
pub fn get_framebuffer() -> Option<&'static LimineFramebufferResponse> {
    unsafe {
        let response = FRAMEBUFFER_REQUEST.response;
        if response.is_null() {
            None
        } else {
            Some(&*response)
        }
    }
}

/// Get HHDM offset from Limine
pub fn get_hhdm_offset() -> Option<u64> {
    unsafe {
        let response = HHDM_REQUEST.response;
        if response.is_null() {
            None
        } else {
            Some((*response).offset)
        }
    }
}

/// Get kernel addresses from Limine
pub fn get_kernel_address() -> Option<&'static LimineKernelAddressResponse> {
    unsafe {
        let response = KERNEL_ADDRESS_REQUEST.response;
        if response.is_null() {
            None
        } else {
            Some(&*response)
        }
    }
}

/// DTB (Device Tree Blob) request — Limine v8 protocol.
/// The response provides the physical address of the DTB passed by firmware.
/// On RISC-V UEFI boots this is the only reliable way to get the DTB address
/// (the `a1` register contains the Limine boot info pointer, not the DTB).
///
/// Request GUID (after LIMINE_COMMON_MAGIC):
///   0x0b40dca86177520e, 0xc8809c1e7bbbde33
/// Verify against: https://github.com/limine-bootloader/limine/blob/v8.x-binary/limine.h
#[repr(C)]
pub struct LimineDtbRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineDtbResponse,
}

unsafe impl Send for LimineDtbRequest {}
unsafe impl Sync for LimineDtbRequest {}

#[repr(C)]
pub struct LimineDtbResponse {
    pub revision: u64,
    /// Physical address of the DTB (may be null if no DTB on this platform).
    pub dtb_ptr: *const u8,
}

#[used]
#[link_section = ".requests"]
static mut DTB_REQUEST: LimineDtbRequest = LimineDtbRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x0b40dca86177520e,
        0xc8809c1e7bbbde33,
    ],
    revision: 0,
    response: core::ptr::null(),
};

/// Return the DTB physical address from the Limine DtbResponse, if present.
///
/// Returns `None` when:
/// - not booted via Limine (OpenSBI direct boot), or
/// - Limine did not populate a DTB response (non-RISC-V or no firmware DTB).
///
/// The kernel's `platform::init` calls this first and falls back to the `a1`
/// register value when it returns `None`.
pub fn get_dtb_ptr() -> Option<usize> {
    // SAFETY: DTB_REQUEST is a Limine protocol request static. The response
    // pointer is written by Limine before the kernel entry point is called,
    // so no data race exists at the point this function is invoked (single
    // hart, no other code running). Null check guards against absent response.
    unsafe {
        let response = DTB_REQUEST.response;
        if response.is_null() {
            return None;
        }
        let dtb = (*response).dtb_ptr;
        if dtb.is_null() {
            return None;
        }
        Some(dtb as usize)
    }
}

/// Limine memory map entry types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum LimineMemoryType {
    Usable = 0,
    Reserved = 1,
    AcpiReclaimable = 2,
    AcpiNvs = 3,
    BadMemory = 4,
    BootloaderReclaimable = 5,
    KernelAndModules = 6,
    Framebuffer = 7,
}

impl LimineMemoryMapEntry {
    pub fn memory_type(&self) -> LimineMemoryType {
        match self.entry_type {
            0 => LimineMemoryType::Usable,
            1 => LimineMemoryType::Reserved,
            2 => LimineMemoryType::AcpiReclaimable,
            3 => LimineMemoryType::AcpiNvs,
            4 => LimineMemoryType::BadMemory,
            5 => LimineMemoryType::BootloaderReclaimable,
            6 => LimineMemoryType::KernelAndModules,
            7 => LimineMemoryType::Framebuffer,
            _ => LimineMemoryType::Reserved,
        }
    }
}
