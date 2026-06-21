// SPDX-License-Identifier: MPL-2.0
// String and memory primitives

#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let s = src as *const u8;
    for i in 0..n {
        *d.add(i) = *s.add(i);
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let s = src as *const u8;
    if (s as usize) < (d as usize) {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *d.add(i) = *s.add(i);
        }
    } else {
        for i in 0..n {
            *d.add(i) = *s.add(i);
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let d = s as *mut u8;
    let v = c as u8;
    for i in 0..n {
        *d.add(i) = v;
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let s1 = core::slice::from_raw_parts(s1 as *const u8, n);
    let s2 = core::slice::from_raw_parts(s2 as *const u8, n);
    for i in 0..n {
        let diff = s1[i] as c_int - s2[i] as c_int;
        if diff != 0 { return diff; }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 { len += 1; }
    len
}

#[no_mangle]
pub unsafe extern "C" fn strcpy(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut i = 0;
    loop {
        let c = *src.add(i);
        *dest.add(i) = c;
        if c == 0 { break; }
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncpy(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let mut i = 0;
    while i < n {
        let c = *src.add(i);
        *dest.add(i) = c;
        if c == 0 {
            while i < n { *dest.add(i) = 0; i += 1; }
            break;
        }
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    let mut i = 0;
    loop {
        let c1 = *s1.add(i) as u8;
        let c2 = *s2.add(i) as u8;
        if c1 != c2 { return c1 as c_int - c2 as c_int; }
        if c1 == 0 { return 0; }
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        let c1 = *s1.add(i) as u8;
        let c2 = *s2.add(i) as u8;
        if c1 != c2 { return c1 as c_int - c2 as c_int; }
        if c1 == 0 { return 0; }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strcat(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let len = strlen(dest);
    strcpy(dest.add(len), src);
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let v = c as u8;
    let mut i = 0;
    loop {
        let b = *s.add(i) as u8;
        if b == v { return s.add(i) as *mut c_char; }
        if b == 0 { return core::ptr::null_mut(); }
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    let v = c as u8;
    let len = strlen(s);
    let mut i = len;
    loop {
        if *s.add(i) as u8 == v { return s.add(i) as *mut c_char; }
        if i == 0 { return core::ptr::null_mut(); }
        i -= 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void {
    let v = c as u8;
    let p = s as *const u8;
    for i in 0..n {
        if *p.add(i) == v { return p.add(i) as *mut c_void; }
    }
    core::ptr::null_mut()
}
