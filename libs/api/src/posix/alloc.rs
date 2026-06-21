// SPDX-License-Identifier: MPL-2.0
// Memory management: malloc / free / realloc / calloc

#![allow(unsafe_code)]

extern crate alloc;

use core::alloc::Layout;
use core::ffi::c_void;
use core::ptr;

#[repr(C)]
struct AllocHeader {
    size: usize,
    magic: usize,
}

const HEADER_MAGIC: usize = 0xDEAD_C0DE;
const HEADER_SIZE: usize = core::mem::size_of::<AllocHeader>();
const ALIGN: usize = 16;

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    malloc_impl(size)
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    free_impl(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    realloc_impl(ptr, size)
}

#[no_mangle]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total_size = match nmemb.checked_mul(size) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };
    let p = malloc(total_size);
    if !p.is_null() {
        core::ptr::write_bytes(p as *mut u8, 0, total_size);
    }
    p
}

pub(super) unsafe fn malloc_impl(size: usize) -> *mut c_void {
    let total = match size.checked_add(HEADER_SIZE) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };
    let layout = match Layout::from_size_align(total, ALIGN) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };
    let raw = alloc::alloc::alloc(layout);
    if raw.is_null() { return ptr::null_mut(); }
    let header = raw as *mut AllocHeader;
    (*header).size = size;
    (*header).magic = HEADER_MAGIC;
    raw.add(HEADER_SIZE) as *mut c_void
}

pub(super) unsafe fn free_impl(ptr: *mut c_void) {
    if ptr.is_null() { return; }
    let raw = (ptr as *mut u8).sub(HEADER_SIZE);
    let header = raw as *mut AllocHeader;
    if (*header).magic != HEADER_MAGIC { return; }
    let total = (*header).size + HEADER_SIZE;
    let layout = Layout::from_size_align_unchecked(total, ALIGN);
    alloc::alloc::dealloc(raw, layout);
}

unsafe fn realloc_impl(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    if ptr.is_null() { return malloc_impl(new_size); }
    if new_size == 0 { free_impl(ptr); return ptr::null_mut(); }
    let raw = (ptr as *mut u8).sub(HEADER_SIZE);
    let header = raw as *mut AllocHeader;
    if (*header).magic != HEADER_MAGIC { return ptr::null_mut(); }
    let old_size = (*header).size;
    let old_layout = Layout::from_size_align_unchecked(old_size + HEADER_SIZE, ALIGN);
    let total_new = match new_size.checked_add(HEADER_SIZE) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };
    let new_raw = alloc::alloc::realloc(raw, old_layout, total_new);
    if new_raw.is_null() { return ptr::null_mut(); }
    let new_header = new_raw as *mut AllocHeader;
    (*new_header).size = new_size;
    (*new_header).magic = HEADER_MAGIC;
    new_raw.add(HEADER_SIZE) as *mut c_void
}

// ---------------------------------------------------------------------------
// C++ operator new/delete stubs
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn _Znwm(size: usize) -> *mut c_void {
    malloc_impl(size)
}

#[no_mangle]
pub unsafe extern "C" fn _ZdlPv(ptr: *mut c_void) {
    free_impl(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn _ZdlPvm(ptr: *mut c_void, _size: usize) {
    free_impl(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn _Znam(size: usize) -> *mut c_void {
    malloc_impl(size)
}

#[no_mangle]
pub unsafe extern "C" fn _ZdaPv(ptr: *mut c_void) {
    free_impl(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn _ZdaPvm(ptr: *mut c_void, _size: usize) {
    free_impl(ptr)
}
