// SPDX-License-Identifier: MPL-2.0

//! POSIX Shim Layer (The Bridge)
//!
//! This module provides a minimal implementation of standard C library functions
//! to support porting Linux CLI applications to ViOS.
//!
//! It translates C calls directly to ViOS system calls or internal mechanisms.

#![allow(unsafe_code)]
#![allow(unused_variables)]
#![allow(non_upper_case_globals)] // For stdin/stdout/stderr

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::{c_char, c_void, c_int, c_double, c_ulong, c_long};
use core::ptr;
use crate::syscall::ViSyscall;

// ----------------------------------------------------------------------------
// 1. Memory Management
// ----------------------------------------------------------------------------

extern "Rust" {
    // We assume the binary linking this (ostd/app) has defined a global allocator.
}

// Internal Header Strategy
#[repr(C)]
struct AllocHeader {
    size: usize,
    magic: usize,
}
const HEADER_MAGIC: usize = 0xDEADC0DE;
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
    let ptr = malloc(total_size);
    if !ptr.is_null() {
        memset(ptr, 0, total_size);
    }
    ptr
}

unsafe fn malloc_impl(size: usize) -> *mut c_void {
    let total_size = match size.checked_add(HEADER_SIZE) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };

    let layout = match Layout::from_size_align(total_size, ALIGN) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };

    let ptr = alloc::alloc::alloc(layout);
    if ptr.is_null() {
        return ptr::null_mut();
    }

    let header = ptr as *mut AllocHeader;
    (*header).size = size;
    (*header).magic = HEADER_MAGIC;

    ptr.add(HEADER_SIZE) as *mut c_void
}

unsafe fn free_impl(ptr: *mut c_void) {
    if ptr.is_null() { return; }

    let real_ptr = (ptr as *mut u8).sub(HEADER_SIZE);
    let header = real_ptr as *mut AllocHeader;

    if (*header).magic != HEADER_MAGIC {
        return;
    }

    let size = (*header).size;
    let total_size = size + HEADER_SIZE;
    let layout = Layout::from_size_align_unchecked(total_size, ALIGN);

    alloc::alloc::dealloc(real_ptr, layout);
}

unsafe fn realloc_impl(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    if ptr.is_null() {
        return malloc_impl(new_size);
    }
    if new_size == 0 {
        free_impl(ptr);
        return ptr::null_mut();
    }

    let real_ptr = (ptr as *mut u8).sub(HEADER_SIZE);
    let header = real_ptr as *mut AllocHeader;

    if (*header).magic != HEADER_MAGIC {
        return ptr::null_mut();
    }

    let old_size = (*header).size;
    let total_old_size = old_size + HEADER_SIZE;
    let old_layout = Layout::from_size_align_unchecked(total_old_size, ALIGN);

    let total_new_size = match new_size.checked_add(HEADER_SIZE) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };

    let new_ptr = alloc::alloc::realloc(real_ptr, old_layout, total_new_size);
    if new_ptr.is_null() {
        return ptr::null_mut();
    }

    let new_header = new_ptr as *mut AllocHeader;
    (*new_header).size = new_size;
    (*new_header).magic = HEADER_MAGIC;

    new_ptr.add(HEADER_SIZE) as *mut c_void
}

// ----------------------------------------------------------------------------
// 2. Strings & Memory (mem*, str*)
// ----------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    ptr::copy_nonoverlapping(src as *const u8, dest as *mut u8, n);
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    ptr::copy(src as *const u8, dest as *mut u8, n);
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    ptr::write_bytes(s as *mut u8, c as u8, n);
    s
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let s1 = core::slice::from_raw_parts(s1 as *const u8, n);
    let s2 = core::slice::from_raw_parts(s2 as *const u8, n);
    for i in 0..n {
        let diff = s1[i] as c_int - s2[i] as c_int;
        if diff != 0 {
            return diff;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void {
    let p = s as *const u8;
    for i in 0..n {
        if *p.add(i) == c as u8 {
            return p.add(i) as *mut c_void;
        }
    }
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
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
            while i < n {
                *dest.add(i) = 0;
                i += 1;
            }
            break;
        }
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strcat(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let dest_len = strlen(dest);
    strcpy(dest.add(dest_len), src);
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncat(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let dest_len = strlen(dest);
    let mut i = 0;
    while i < n {
        let c = *src.add(i);
        if c == 0 { break; }
        *dest.add(dest_len + i) = c;
        i += 1;
    }
    *dest.add(dest_len + i) = 0;
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    let mut i = 0;
    loop {
        let c1 = *s1.add(i) as u8;
        let c2 = *s2.add(i) as u8;
        if c1 != c2 {
            return (c1 as c_int) - (c2 as c_int);
        }
        if c1 == 0 {
            return 0;
        }
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        let c1 = *s1.add(i) as u8;
        let c2 = *s2.add(i) as u8;
        if c1 != c2 {
            return (c1 as c_int) - (c2 as c_int);
        }
        if c1 == 0 {
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let mut i = 0;
    loop {
        let ch = *s.add(i);
        if ch == c as c_char {
            return s.add(i) as *mut c_char;
        }
        if ch == 0 {
            return ptr::null_mut();
        }
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    let mut last = ptr::null_mut();
    let mut i = 0;
    loop {
        let ch = *s.add(i);
        if ch == c as c_char {
            last = s.add(i) as *mut c_char;
        }
        if ch == 0 {
            return last;
        }
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    let needle_len = strlen(needle);
    if needle_len == 0 {
        return haystack as *mut c_char;
    }
    let mut h = haystack;
    while *h != 0 {
        if strncmp(h, needle, needle_len) == 0 {
            return h as *mut c_char;
        }
        h = h.add(1);
    }
    ptr::null_mut()
}

static mut STRTOK_STATE: *mut c_char = ptr::null_mut();

#[no_mangle]
pub unsafe extern "C" fn strtok(s: *mut c_char, delim: *const c_char) -> *mut c_char {
    let mut input = s;
    if input.is_null() {
        input = STRTOK_STATE;
    }
    if input.is_null() {
        return ptr::null_mut();
    }

    // Skip delimiters
    while *input != 0 && !strchr(delim, *input as c_int).is_null() {
        input = input.add(1);
    }

    if *input == 0 {
        STRTOK_STATE = ptr::null_mut();
        return ptr::null_mut();
    }

    let start = input;
    // Find next delimiter
    while *input != 0 && strchr(delim, *input as c_int).is_null() {
        input = input.add(1);
    }

    if *input != 0 {
        *input = 0;
        STRTOK_STATE = input.add(1);
    } else {
        STRTOK_STATE = ptr::null_mut();
    }

    start
}

#[no_mangle]
pub unsafe extern "C" fn strpbrk(s: *const c_char, accept: *const c_char) -> *mut c_char {
    let mut s_ptr = s;
    while *s_ptr != 0 {
        if !strchr(accept, *s_ptr as c_int).is_null() {
            return s_ptr as *mut c_char;
        }
        s_ptr = s_ptr.add(1);
    }
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn strspn(s: *const c_char, accept: *const c_char) -> usize {
    let mut count = 0;
    while *s.add(count) != 0 {
        if strchr(accept, *s.add(count) as c_int).is_null() {
            break;
        }
        count += 1;
    }
    count
}

#[no_mangle]
pub unsafe extern "C" fn strcspn(s: *const c_char, reject: *const c_char) -> usize {
    let mut count = 0;
    while *s.add(count) != 0 {
        if !strchr(reject, *s.add(count) as c_int).is_null() {
            break;
        }
        count += 1;
    }
    count
}

// ----------------------------------------------------------------------------
// 3. String Conversion (strtol, etc)
// ----------------------------------------------------------------------------

// Simple implementation ignoring locale and complex cases
#[no_mangle]
pub unsafe extern "C" fn strtol(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long {
    let mut s = nptr;
    // Skip whitespace
    while isspace(*s as c_int) != 0 { s = s.add(1); }

    let mut neg = false;
    if *s as u8 == b'-' {
        neg = true;
        s = s.add(1);
    } else if *s as u8 == b'+' {
        s = s.add(1);
    }

    let mut b = base;
    if base == 0 {
        if *s as u8 == b'0' {
            if *s.add(1) as u8 == b'x' || *s.add(1) as u8 == b'X' {
                b = 16;
                s = s.add(2);
            } else {
                b = 8;
            }
        } else {
            b = 10;
        }
    } else if base == 16 {
        if *s as u8 == b'0' && (*s.add(1) as u8 == b'x' || *s.add(1) as u8 == b'X') {
            s = s.add(2);
        }
    }

    let mut acc: c_long = 0;
    loop {
        let c = *s as u8;
        let val = if c >= b'0' && c <= b'9' {
            c - b'0'
        } else if c >= b'a' && c <= b'z' {
            c - b'a' + 10
        } else if c >= b'A' && c <= b'Z' {
            c - b'A' + 10
        } else {
            break;
        };

        if val >= b as u8 { break; }

        // No overflow check for brevity/shim
        acc = acc.wrapping_mul(b as c_long).wrapping_add(val as c_long);
        s = s.add(1);
    }

    if !endptr.is_null() {
        *endptr = s as *mut c_char;
    }

    if neg { -acc } else { acc }
}

#[no_mangle]
pub unsafe extern "C" fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong {
    strtol(nptr, endptr, base) as c_ulong // Reusing logic
}

#[no_mangle]
pub unsafe extern "C" fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double {
    // Very basic float parser: D.D
    // Does not support exponents "e" yet for minimal shim
    let mut s = nptr;
    while isspace(*s as c_int) != 0 { s = s.add(1); }

    let mut neg = false;
    if *s as u8 == b'-' {
        neg = true;
        s = s.add(1);
    } else if *s as u8 == b'+' {
        s = s.add(1);
    }

    let mut acc: c_double = 0.0;
    while isdigit(*s as c_int) != 0 {
        acc = acc * 10.0 + ((*s as u8 - b'0') as c_double);
        s = s.add(1);
    }

    if *s as u8 == b'.' {
        s = s.add(1);
        let mut factor = 0.1;
        while isdigit(*s as c_int) != 0 {
            acc = acc + ((*s as u8 - b'0') as c_double) * factor;
            factor *= 0.1;
            s = s.add(1);
        }
    }

    if !endptr.is_null() {
        *endptr = s as *mut c_char;
    }

    if neg { -acc } else { acc }
}

// ----------------------------------------------------------------------------
// 4. Ctype
// ----------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn isalpha(c: c_int) -> c_int {
    if (c >= 'a' as c_int && c <= 'z' as c_int) || (c >= 'A' as c_int && c <= 'Z' as c_int) { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn isdigit(c: c_int) -> c_int {
    if c >= '0' as c_int && c <= '9' as c_int { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn isalnum(c: c_int) -> c_int {
    if isalpha(c) != 0 || isdigit(c) != 0 { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn isspace(c: c_int) -> c_int {
    match c as u8 as char {
        ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' => 1,
        _ => 0,
    }
}
#[no_mangle]
pub extern "C" fn ispunct(c: c_int) -> c_int {
    if isalpha(c) == 0 && isdigit(c) == 0 && isspace(c) == 0 && c != 0 { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn iscntrl(c: c_int) -> c_int {
    if (c >= 0 && c <= 31) || c == 127 { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn isxdigit(c: c_int) -> c_int {
    if isdigit(c) != 0 || (c >= 'a' as c_int && c <= 'f' as c_int) || (c >= 'A' as c_int && c <= 'F' as c_int) { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn tolower(c: c_int) -> c_int {
    if c >= 'A' as c_int && c <= 'Z' as c_int { c + 32 } else { c }
}
#[no_mangle]
pub extern "C" fn toupper(c: c_int) -> c_int {
    if c >= 'a' as c_int && c <= 'z' as c_int { c - 32 } else { c }
}

// ----------------------------------------------------------------------------
// 5. System Call Helper
// ----------------------------------------------------------------------------
#[inline(always)]
unsafe fn raw_syscall(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let mut ret: isize;
    core::arch::asm!(
        "ecall",
        inlateout("a0") a0 => ret,
        in("a1") a1,
        in("a2") a2,
        in("a3") a3,
        in("a7") (id as usize),
        options(nostack, preserves_flags)
    );
    ret
}

// ----------------------------------------------------------------------------
// 6. I/O (printf, puts)
// ----------------------------------------------------------------------------

// Standard Streams (FD + 1)
// FD 0 -> 1, FD 1 -> 2, FD 2 -> 3
#[no_mangle]
pub static mut stdin: *mut c_void = 1 as *mut c_void;
#[no_mangle]
pub static mut stdout: *mut c_void = 2 as *mut c_void;
#[no_mangle]
pub static mut stderr: *mut c_void = 3 as *mut c_void;

#[no_mangle]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    if s.is_null() { return -1; }
    let len = strlen(s);
    raw_syscall(ViSyscall::Log, s as usize, len, 0, 0);
    let newline = "\n";
    raw_syscall(ViSyscall::Log, newline.as_ptr() as usize, 1, 0, 0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn putchar(c: c_int) -> c_int {
    let buf = [c as u8];
    // FD 1 (Stdout)
    raw_syscall(ViSyscall::Write, 1, buf.as_ptr() as usize, 1, 0);
    c
}

#[no_mangle]
pub unsafe extern "C" fn getchar() -> c_int {
    let mut buf = [0u8; 1];
    if raw_syscall(ViSyscall::Read, 0, buf.as_mut_ptr() as usize, 1, 0) > 0 {
        buf[0] as c_int
    } else {
        -1 // EOF
    }
}

// Minimal helpers for printing numbers
unsafe fn print_dec(mut n: isize) {
    let mut buf = [0u8; 32];
    let mut i = 31;
    let is_neg = n < 0;
    if is_neg { n = -n; }

    if n == 0 {
        buf[i] = b'0';
        i -= 1;
    } else {
        while n > 0 {
            buf[i] = (n % 10) as u8 + b'0';
            n /= 10;
            i -= 1;
        }
    }
    if is_neg {
        buf[i] = b'-';
        i -= 1;
    }
    i += 1;
    raw_syscall(ViSyscall::Log, buf.as_ptr().add(i) as usize, 32 - i, 0, 0);
}

unsafe fn print_hex(mut n: usize) {
    let mut buf = [0u8; 32];
    let mut i = 31;
    let hex = b"0123456789abcdef";

    if n == 0 {
        buf[i] = b'0';
        i -= 1;
    } else {
        while n > 0 {
            buf[i] = hex[n % 16];
            n /= 16;
            i -= 1;
        }
    }
    i += 1;
    raw_syscall(ViSyscall::Log, buf.as_ptr().add(i) as usize, 32 - i, 0, 0);
}

#[no_mangle]
pub unsafe extern "C" fn printf(fmt: *const c_char, mut args: ...) -> c_int {
    if fmt.is_null() { return 0; }

    let mut i = 0;
    loop {
        let c = *fmt.add(i) as u8;
        if c == 0 { break; }

        if c == b'%' {
            i += 1;
            let next = *fmt.add(i) as u8;
            match next {
                b'd' => {
                    let val = args.arg::<c_int>();
                    print_dec(val as isize);
                }
                b'x' => {
                    let val = args.arg::<c_int>();
                    print_hex(val as usize);
                }
                b's' => {
                    let val = args.arg::<*const c_char>();
                    let len = strlen(val);
                    raw_syscall(ViSyscall::Log, val as usize, len, 0, 0);
                }
                b'c' => {
                    let val = args.arg::<c_int>();
                    let buf = [val as u8];
                    raw_syscall(ViSyscall::Log, buf.as_ptr() as usize, 1, 0, 0);
                }
                b'%' => {
                    let buf = [b'%'];
                    raw_syscall(ViSyscall::Log, buf.as_ptr() as usize, 1, 0, 0);
                }
                _ => {
                    let buf = [b'%', next];
                    raw_syscall(ViSyscall::Log, buf.as_ptr() as usize, 2, 0, 0);
                }
            }
        } else {
            let buf = [c];
            raw_syscall(ViSyscall::Log, buf.as_ptr() as usize, 1, 0, 0);
        }
        i += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sprintf(str: *mut c_char, format: *const c_char, args: ...) -> c_int {
    // Stub: Writes empty string
    *str = 0;
    0
}

#[no_mangle]
pub unsafe extern "C" fn snprintf(str: *mut c_char, size: usize, format: *const c_char, args: ...) -> c_int {
    if size > 0 {
        *str = 0;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn vsnprintf(str: *mut c_char, size: usize, format: *const c_char, mut args: ...) -> c_int {
    // Implementing a full vsnprintf in shim is hard without std::fmt.
    // This is often required by scripting languages.
    // For now, minimal stub that ensures null termination.
    if size > 0 {
        *str = 0;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn fputs(s: *const c_char, stream: *mut c_void) -> c_int {
    if stream.is_null() { return -1; }
    let fd = (stream as usize) - 1;
    let len = strlen(s);
    raw_syscall(ViSyscall::Write, fd, s as usize, len, 0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn fgets(s: *mut c_char, size: c_int, stream: *mut c_void) -> *mut c_char {
    if stream.is_null() || size <= 0 { return ptr::null_mut(); }
    let fd = (stream as usize) - 1;

    // Read char by char until newline or size-1
    let mut i = 0;
    while i < (size - 1) {
        let mut buf = [0u8; 1];
        if raw_syscall(ViSyscall::Read, fd, buf.as_mut_ptr() as usize, 1, 0) <= 0 {
            break;
        }
        *s.add(i as usize) = buf[0] as c_char;
        i += 1;
        if buf[0] == b'\n' {
            break;
        }
    }
    *s.add(i as usize) = 0;
    if i == 0 { return ptr::null_mut(); }
    s
}

// ----------------------------------------------------------------------------
// 7. File I/O
// ----------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn fopen(filename: *const c_char, _mode: *const c_char) -> *mut c_void {
    if filename.is_null() { return ptr::null_mut(); }
    let len = strlen(filename);

    // Syscall Open (101)
    let ret = raw_syscall(ViSyscall::Open, filename as usize, len, 0, 0);
    if ret < 0 {
        return ptr::null_mut();
    }
    // Return FD + 1
    ((ret + 1) as usize) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn fclose(stream: *mut c_void) -> c_int {
    if stream.is_null() { return -1; }
    let fd = (stream as usize) - 1;
    raw_syscall(ViSyscall::Close, fd, 0, 0, 0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize {
    if stream.is_null() || ptr.is_null() { return 0; }
    let fd = (stream as usize) - 1;
    let bytes_to_read = size * nmemb;
    let ret = raw_syscall(ViSyscall::Read, fd, ptr as usize, bytes_to_read, 0);
    if ret < 0 { return 0; }
    (ret as usize) / size
}

#[no_mangle]
pub unsafe extern "C" fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize {
    if stream.is_null() || ptr.is_null() { return 0; }
    let fd = (stream as usize) - 1;
    let bytes_to_write = size * nmemb;
    let ret = raw_syscall(ViSyscall::Write, fd, ptr as usize, bytes_to_write, 0);
    if ret < 0 { return 0; }
    (ret as usize) / size
}

#[no_mangle]
pub unsafe extern "C" fn fseek(_stream: *mut c_void, _offset: c_ulong, _whence: c_int) -> c_int {
    // TODO
    0
}
#[no_mangle]
pub unsafe extern "C" fn ftell(_stream: *mut c_void) -> c_ulong {
    // TODO
    0
}
#[no_mangle]
pub unsafe extern "C" fn rewind(stream: *mut c_void) {
    fseek(stream, 0, 0); // SEEK_SET = 0
}
#[no_mangle]
pub unsafe extern "C" fn fflush(_stream: *mut c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn getc(_stream: *mut c_void) -> c_int {
    // Read 1 byte
    let mut buf = [0u8; 1];
    if fread(buf.as_mut_ptr() as *mut c_void, 1, 1, _stream) == 1 {
        buf[0] as c_int
    } else {
        -1 // EOF
    }
}
#[no_mangle]
pub unsafe extern "C" fn ungetc(_c: c_int, _stream: *mut c_void) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn feof(_stream: *mut c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn ferror(_stream: *mut c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn remove(_filename: *const c_char) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn rename(_old: *const c_char, _new: *const c_char) -> c_int {
    -1
}

// ----------------------------------------------------------------------------
// 8. Math (via libm)
// ----------------------------------------------------------------------------

macro_rules! math_shim {
    ($name:ident, $arg:ty) => {
        #[no_mangle]
        pub extern "C" fn $name(n: $arg) -> $arg {
            libm::$name(n)
        }
    };
    ($name:ident, $arg1:ty, $arg2:ty) => {
        #[no_mangle]
        pub extern "C" fn $name(a: $arg1, b: $arg2) -> $arg1 {
            libm::$name(a, b)
        }
    };
}

math_shim!(acos, c_double);
math_shim!(asin, c_double);
math_shim!(atan, c_double);
math_shim!(atan2, c_double, c_double);
math_shim!(ceil, c_double);
math_shim!(cos, c_double);
math_shim!(cosh, c_double);
math_shim!(exp, c_double);
math_shim!(fabs, c_double);
math_shim!(floor, c_double);
math_shim!(fmod, c_double, c_double);
math_shim!(log, c_double);
math_shim!(log10, c_double);
math_shim!(pow, c_double, c_double);
math_shim!(sin, c_double);
math_shim!(sinh, c_double);
math_shim!(sqrt, c_double);
math_shim!(tan, c_double);
math_shim!(tanh, c_double);

#[no_mangle]
pub extern "C" fn ldexp(x: c_double, n: c_int) -> c_double {
    libm::ldexp(x, n)
}

#[no_mangle]
pub extern "C" fn frexp(x: c_double, exp: *mut c_int) -> c_double {
    let (fr, ex) = libm::frexp(x);
    unsafe { *exp = ex; }
    fr
}
#[no_mangle]
pub extern "C" fn modf(x: c_double, iptr: *mut c_double) -> c_double {
    let (fr, i) = libm::modf(x);
    unsafe { *iptr = i; }
    fr
}

// ----------------------------------------------------------------------------
// 9. Error Handling & Time & Control
// ----------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn setjmp(_env: *mut c_void) -> c_int {
    // TODO: Assembly required for register saving to support pcall in Lua
    0
}
#[no_mangle]
pub unsafe extern "C" fn longjmp(_env: *mut c_void, _val: c_int) {
    // TODO: Assembly required for register restoring to support pcall in Lua
    loop {}
}

#[no_mangle]
pub unsafe extern "C" fn time(_tloc: *mut c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn clock() -> c_ulong {
    0
}
#[no_mangle]
pub unsafe extern "C" fn difftime(_time1: c_int, _time0: c_int) -> c_double {
    0.0
}
#[no_mangle]
pub unsafe extern "C" fn exit(status: c_int) -> ! {
    raw_syscall(ViSyscall::Exit, status as usize, 0, 0, 0);
    loop {}
}

#[no_mangle]
pub unsafe extern "C" fn abort() -> ! {
    exit(-1)
}
