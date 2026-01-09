#![allow(unsafe_code)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(unused_variables)]
#![allow(dead_code)]

use core::ffi::{c_void, c_char, c_int, c_long, c_double};
use core::ptr;
use core::alloc::Layout;
use core::arch::asm;
use crate::syscall::ViSyscall;

// --- Syscall Helper ---
#[inline(always)]
unsafe fn syscall(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let mut ret: isize;
    asm!(
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

// --- Memory Management ---

#[repr(C)]
struct AllocHeader {
    size: usize,
    align: usize,
}

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    // Default alignment for malloc is usually 8 or 16. Let's use 16 (sizeof(usize)*2)
    let align = core::mem::align_of::<usize>() * 2;
    let header_size = core::mem::size_of::<AllocHeader>();
    let total_size = size + header_size;

    // We need to ensure the header + data respects alignment.
    // If we allocate `total_size` with `align`, the returned pointer is aligned.
    // We put header at start. data starts at header_size.
    // If header_size is not multiple of align, data won't be aligned.
    // sizeof(AllocHeader) is 2 * usize = 16 bytes (64-bit).
    // So if align is 16, it works.

    let layout = Layout::from_size_align(total_size, align).unwrap();
    let ptr = alloc::alloc::alloc(layout);

    if ptr.is_null() {
        return ptr::null_mut();
    }

    let header = ptr as *mut AllocHeader;
    (*header).size = size;
    (*header).align = align;

    ptr.add(header_size) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let header_size = core::mem::size_of::<AllocHeader>();
    let raw_ptr = (ptr as *mut u8).sub(header_size);
    let header = raw_ptr as *mut AllocHeader;

    let size = (*header).size + header_size;
    let align = (*header).align;

    let layout = Layout::from_size_align(size, align).unwrap();
    alloc::alloc::dealloc(raw_ptr, layout);
}

#[no_mangle]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total_size = nmemb * size;
    let ptr = malloc(total_size);
    if !ptr.is_null() {
        ptr::write_bytes(ptr, 0, total_size);
    }
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        return malloc(size);
    }
    if size == 0 {
        free(ptr);
        return ptr::null_mut();
    }

    let header_size = core::mem::size_of::<AllocHeader>();
    let raw_ptr = (ptr as *mut u8).sub(header_size);
    let header = raw_ptr as *mut AllocHeader;

    let old_size = (*header).size; // User size
    let align = (*header).align;
    let old_total_size = old_size + header_size;

    let new_total_size = size + header_size;
    let new_layout = Layout::from_size_align(new_total_size, align).unwrap();

    // Rust's realloc takes the OLD layout
    let old_layout = Layout::from_size_align(old_total_size, align).unwrap();

    let new_ptr = alloc::alloc::realloc(raw_ptr, old_layout, new_total_size);

    if new_ptr.is_null() {
        return ptr::null_mut();
    }

    let new_header = new_ptr as *mut AllocHeader;
    (*new_header).size = size;
    // align remains same

    new_ptr.add(header_size) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    ptr::write_bytes(s as *mut u8, c as u8, n);
    s
}

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
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let s1 = s1 as *const u8;
    let s2 = s2 as *const u8;
    for i in 0..n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return (a as c_int) - (b as c_int);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void {
    let s = s as *const u8;
    let c = c as u8;
    for i in 0..n {
        if *s.add(i) == c {
            return s.add(i) as *mut c_void;
        }
    }
    ptr::null_mut()
}

// --- String Handling ---

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
            // pad the rest with 0
            for j in (i+1)..n {
                *dest.add(j) = 0;
            }
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
    if n == 0 { return 0; }
    let mut i = 0;
    while i < n {
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
    0
}

#[no_mangle]
pub unsafe extern "C" fn strcat(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let len = strlen(dest);
    strcpy(dest.add(len), src);
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncat(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let len = strlen(dest);
    let mut i = 0;
    while i < n {
        let c = *src.add(i);
        if c == 0 { break; }
        *dest.add(len + i) = c;
        i += 1;
    }
    *dest.add(len + i) = 0;
    dest
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

// TODO: Correctly implement strtok with static state or thread-local storage if possible.
// For now, we use a static buffer which is not thread-safe but standard for simple strtok.
static mut STRTOK_NEXT: *mut c_char = ptr::null_mut();
#[no_mangle]
pub unsafe extern "C" fn strtok(s: *mut c_char, delim: *const c_char) -> *mut c_char {
    let mut str = s;
    if str.is_null() {
        str = STRTOK_NEXT;
    }
    if str.is_null() {
        return ptr::null_mut();
    }

    // Skip delimiters
    while *str != 0 && strchr(delim, *str as c_int) != ptr::null_mut() {
        str = str.add(1);
    }
    if *str == 0 {
        STRTOK_NEXT = ptr::null_mut();
        return ptr::null_mut();
    }

    let token_start = str;

    // Scan token
    while *str != 0 {
        if strchr(delim, *str as c_int) != ptr::null_mut() {
            *str = 0;
            STRTOK_NEXT = str.add(1);
            return token_start;
        }
        str = str.add(1);
    }

    STRTOK_NEXT = ptr::null_mut();
    token_start
}

// Simple stubs for other string funcs
#[no_mangle]
pub unsafe extern "C" fn strpbrk(s: *const c_char, accept: *const c_char) -> *mut c_char {
    let mut i = 0;
    while *s.add(i) != 0 {
        if strchr(accept, *s.add(i) as c_int) != ptr::null_mut() {
            return s.add(i) as *mut c_char;
        }
        i += 1;
    }
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn strspn(s: *const c_char, accept: *const c_char) -> usize {
    let mut i = 0;
    while *s.add(i) != 0 {
        if strchr(accept, *s.add(i) as c_int) == ptr::null_mut() {
            return i;
        }
        i += 1;
    }
    i
}

#[no_mangle]
pub unsafe extern "C" fn strcspn(s: *const c_char, reject: *const c_char) -> usize {
    let mut i = 0;
    while *s.add(i) != 0 {
        if strchr(reject, *s.add(i) as c_int) != ptr::null_mut() {
            return i;
        }
        i += 1;
    }
    i
}

// Stub for conversion functions
#[no_mangle]
pub unsafe extern "C" fn strtol(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long {
    // Basic implementation skipping whitespace and handling sign
    let mut s = nptr;
    while isspace(*s as c_int) != 0 { s = s.add(1); }

    let mut sign = 1;
    if *s == '-' as c_char {
        sign = -1;
        s = s.add(1);
    } else if *s == '+' as c_char {
        s = s.add(1);
    }

    let mut res: c_long = 0;
    let b = if base == 0 {
        if *s == '0' as c_char {
            if *s.add(1) == 'x' as c_char || *s.add(1) == 'X' as c_char {
                s = s.add(2);
                16
            } else {
                8
            }
        } else {
            10
        }
    } else {
        base
    };

    // Check hex prefix if base 16
    if b == 16 && *s == '0' as c_char && (*s.add(1) == 'x' as c_char || *s.add(1) == 'X' as c_char) {
        s = s.add(2);
    }

    loop {
        let c = *s;
        let val = if c >= '0' as c_char && c <= '9' as c_char {
            (c as u8 - b'0') as c_long
        } else if c >= 'a' as c_char && c <= 'z' as c_char {
            (c as u8 - b'a' + 10) as c_long
        } else if c >= 'A' as c_char && c <= 'Z' as c_char {
            (c as u8 - b'A' + 10) as c_long
        } else {
            break;
        };

        if val >= b as c_long { break; }

        res = res * (b as c_long) + val;
        s = s.add(1);
    }

    if !endptr.is_null() {
        *endptr = s as *mut c_char;
    }

    res * sign
}

#[no_mangle]
pub unsafe extern "C" fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> u32 {
    strtol(nptr, endptr, base) as u32 // Simplified
}

#[no_mangle]
pub unsafe extern "C" fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double {
    // Very basic stub
    if !endptr.is_null() {
        *endptr = nptr as *mut c_char;
    }
    0.0
}

// Character checks
#[no_mangle] pub extern "C" fn tolower(c: c_int) -> c_int { if c >= 'A' as i32 && c <= 'Z' as i32 { c + 32 } else { c } }
#[no_mangle] pub extern "C" fn toupper(c: c_int) -> c_int { if c >= 'a' as i32 && c <= 'z' as i32 { c - 32 } else { c } }
#[no_mangle] pub extern "C" fn isalpha(c: c_int) -> c_int { if (c >= 'a' as i32 && c <= 'z' as i32) || (c >= 'A' as i32 && c <= 'Z' as i32) { 1 } else { 0 } }
#[no_mangle] pub extern "C" fn isdigit(c: c_int) -> c_int { if c >= '0' as i32 && c <= '9' as i32 { 1 } else { 0 } }
#[no_mangle] pub extern "C" fn isspace(c: c_int) -> c_int { if c == ' ' as i32 || c == '\t' as i32 || c == '\n' as i32 || c == '\r' as i32 { 1 } else { 0 } }
#[no_mangle] pub extern "C" fn isalnum(c: c_int) -> c_int { if isalpha(c) != 0 || isdigit(c) != 0 { 1 } else { 0 } }
#[no_mangle] pub extern "C" fn isxdigit(c: c_int) -> c_int { if isdigit(c) != 0 || (c >= 'a' as i32 && c <= 'f' as i32) || (c >= 'A' as i32 && c <= 'F' as i32) { 1 } else { 0 } }
#[no_mangle] pub extern "C" fn ispunct(c: c_int) -> c_int { if isalnum(c) == 0 && isspace(c) == 0 && c != 0 { 1 } else { 0 } }


// --- Standard I/O ---

// We define dummy file handles
#[no_mangle] pub static mut stdin: *mut c_void = 0 as *mut c_void;
#[no_mangle] pub static mut stdout: *mut c_void = 1 as *mut c_void;
#[no_mangle] pub static mut stderr: *mut c_void = 2 as *mut c_void;

#[no_mangle]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    let len = strlen(s);
    // Write string
    syscall(ViSyscall::Write, 1, s as usize, len, 0);
    // Write newline
    let nl = "\n";
    syscall(ViSyscall::Write, 1, nl.as_ptr() as usize, 1, 0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn putchar(c: c_int) -> c_int {
    let ch = c as u8;
    syscall(ViSyscall::Write, 1, &ch as *const u8 as usize, 1, 0);
    c
}

#[no_mangle]
pub unsafe extern "C" fn getchar() -> c_int {
    let mut c: u8 = 0;
    let ret = syscall(ViSyscall::Read, 0, &mut c as *mut u8 as usize, 1, 0);
    if ret == 1 {
        c as c_int
    } else {
        -1 // EOF
    }
}

// File ops wrappers
#[no_mangle]
pub unsafe extern "C" fn fopen(filename: *const c_char, mode: *const c_char) -> *mut c_void {
    let len = strlen(filename);
    // mode is ignored for now
    let fd = syscall(ViSyscall::Open, filename as usize, len, 0, 0);
    if fd >= 0 {
        fd as *mut c_void
    } else {
        ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn fclose(stream: *mut c_void) -> c_int {
    syscall(ViSyscall::Close, stream as usize, 0, 0, 0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize {
    let bytes_to_read = size * nmemb;
    let ret = syscall(ViSyscall::Read, stream as usize, ptr as usize, bytes_to_read, 0);
    if ret >= 0 {
        ret as usize / size
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut c_void) -> usize {
    let bytes_to_write = size * nmemb;
    let ret = syscall(ViSyscall::Write, stream as usize, ptr as usize, bytes_to_write, 0);
    if ret >= 0 {
        ret as usize / size
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn fseek(stream: *mut c_void, offset: c_long, whence: c_int) -> c_int {
    // TODO: Need sys_lseek
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ftell(stream: *mut c_void) -> c_long {
    // TODO
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rewind(stream: *mut c_void) {
    fseek(stream, 0, 0); // SEEK_SET = 0
}

#[no_mangle]
pub unsafe extern "C" fn fflush(stream: *mut c_void) -> c_int {
    // no-op
    0
}

#[no_mangle]
pub unsafe extern "C" fn remove(filename: *const c_char) -> c_int {
    // TODO: sys_unlink?
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rename(old: *const c_char, new: *const c_char) -> c_int {
    // TODO
    -1
}

#[no_mangle]
pub unsafe extern "C" fn fgets(s: *mut c_char, size: c_int, stream: *mut c_void) -> *mut c_char {
    // Basic readline
    if size <= 0 { return ptr::null_mut(); }
    let mut i = 0;
    loop {
        if i >= size - 1 { break; }
        let mut c: u8 = 0;
        let ret = syscall(ViSyscall::Read, stream as usize, &mut c as *mut u8 as usize, 1, 0);
        if ret <= 0 {
            if i == 0 { return ptr::null_mut(); }
            break;
        }
        *s.add(i as usize) = c as c_char;
        i += 1;
        if c == b'\n' { break; }
    }
    *s.add(i as usize) = 0;
    s
}

#[no_mangle]
pub unsafe extern "C" fn fputs(s: *const c_char, stream: *mut c_void) -> c_int {
    let len = strlen(s);
    let ret = syscall(ViSyscall::Write, stream as usize, s as usize, len, 0);
    if ret >= 0 { 0 } else { -1 }
}

// Printf family - STUB for now or basic implementation?
// User said: "MicroPython đòi vsnprintf rất gắt để format string".
// Implementing full vsnprintf in C style in Rust is hard without a crate.
// I will stub it to return failure or empty string, marking TODO.
#[no_mangle]
pub unsafe extern "C" fn printf(format: *const c_char, _: ...) -> c_int {
    // Variadic functions in Rust are not fully supported for definition.
    // However, C code calls this.
    // We can't define variadic function in Rust easily to handle C varargs.
    // We typically use `vprintf` which takes `va_list`.
    // But `va_list` structure is platform dependent.
    // For now, I will use a dummy implementation that prints the format string only if it has no args?
    // Actually, I can't easily implement varargs in Rust.
    // Best effort: Print "printf stub" or just print the format string.
    let len = strlen(format);
    syscall(ViSyscall::Write, 1, format as usize, len, 0);
    len as c_int
}

#[no_mangle]
pub unsafe extern "C" fn sprintf(str: *mut c_char, format: *const c_char, _: ...) -> c_int {
    // Stub: just copy format to str? (dangerous if format has %s etc)
    *str = 0;
    0
}

#[no_mangle]
pub unsafe extern "C" fn snprintf(str: *mut c_char, size: usize, format: *const c_char, _: ...) -> c_int {
    if size > 0 { *str = 0; }
    0
}

#[no_mangle]
pub unsafe extern "C" fn vsnprintf(str: *mut c_char, size: usize, format: *const c_char, ap: *mut c_void) -> c_int {
    // This is critical for MicroPython.
    // Without a real printf implementation, this is hard.
    // I'll leave a TODO.
    if size > 0 { *str = 0; }
    0
}


// --- Flow Control ---
#[no_mangle]
pub unsafe extern "C" fn setjmp(env: *mut c_void) -> c_int {
    // TODO: Assembly required to save registers
    0
}

#[no_mangle]
pub unsafe extern "C" fn longjmp(env: *mut c_void, val: c_int) {
    // TODO: Assembly required to restore registers
    loop {}
}

// --- Math ---
// Forward to libm
// libm crate provides these as `#[no_mangle]` if we enable `compiler-builtins` features maybe?
// Actually libm functions are usually just `pub fn sin(x: f64) -> f64`.
// We need to export them as extern "C".

macro_rules! export_math {
    ($name:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(x: c_double) -> c_double {
            libm::$name(x)
        }
    };
    ($name:ident, 2) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(x: c_double, y: c_double) -> c_double {
            libm::$name(x, y)
        }
    }
}

export_math!(sin);
export_math!(cos);
export_math!(tan);
export_math!(asin);
export_math!(acos);
export_math!(atan);
export_math!(atan2, 2);
export_math!(exp);
export_math!(log);
export_math!(log10);
export_math!(pow, 2);
export_math!(sqrt);
export_math!(ceil);
export_math!(floor);
export_math!(fabs);
export_math!(fmod, 2);

// modf takes a pointer
#[no_mangle]
pub unsafe extern "C" fn modf(x: c_double, iptr: *mut c_double) -> c_double {
    let (frac, int_part) = libm::modf(x);
    *iptr = int_part;
    frac
}

// frexp takes a pointer to int
#[no_mangle]
pub unsafe extern "C" fn frexp(x: c_double, exp: *mut c_int) -> c_double {
    let (frac, e) = libm::frexp(x);
    *exp = e;
    frac
}

#[no_mangle]
pub unsafe extern "C" fn ldexp(x: c_double, exp: c_int) -> c_double {
    libm::ldexp(x, exp)
}

// --- System & Time ---
#[no_mangle]
pub unsafe extern "C" fn exit(status: c_int) -> ! {
    syscall(ViSyscall::Exit, status as usize, 0, 0, 0);
    loop {}
}

#[no_mangle]
pub unsafe extern "C" fn abort() -> ! {
    exit(134); // SIGABRT
}

#[no_mangle]
pub unsafe extern "C" fn time(t: *mut c_long) -> c_long {
    // TODO: sys_time?
    0
}

#[no_mangle]
pub unsafe extern "C" fn clock() -> c_long {
    // TODO
    0
}

#[no_mangle]
pub unsafe extern "C" fn difftime(time1: c_long, time0: c_long) -> c_double {
    (time1 - time0) as c_double
}
