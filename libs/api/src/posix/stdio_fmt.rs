// SPDX-License-Identifier: MPL-2.0
// vsnprintf implementation — used internally by stdio.rs public API.
//
// VaList API (nightly-2026-05-01): one lifetime, method is `.next_arg::<T>()`.

#![allow(unsafe_code)]

use core::ffi::VaList;

// ---------------------------------------------------------------------------
// Number-to-ASCII helpers
// ---------------------------------------------------------------------------

/// Writes `n` in the given base into `tmp[0..len]`, returns `len`.
fn fmt_uint(tmp: &mut [u8; 24], mut n: u64, base: u64, upper: bool) -> usize {
    let digits = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    let mut rev = [0u8; 24];
    let mut len = 0usize;
    if n == 0 { rev[0] = b'0'; len = 1; }
    while n > 0 { rev[len] = digits[(n % base) as usize]; n /= base; len += 1; }
    for i in 0..len { tmp[i] = rev[len - 1 - i]; }
    len
}

fn fmt_int(tmp: &mut [u8; 24], n: i64) -> usize {
    if n < 0 {
        tmp[0] = b'-';
        let mut sub = [0u8; 24];
        let l = fmt_uint(&mut sub, n.unsigned_abs(), 10, false);
        tmp[1..1 + l].copy_from_slice(&sub[..l]);
        l + 1
    } else {
        fmt_uint(tmp, n as u64, 10, false)
    }
}

/// Minimal %f formatter: prints `prec` decimal places.
pub(super) fn fmt_float(tmp: &mut [u8; 64], val: f64, prec: usize) -> usize {
    if val.is_nan() { tmp[..3].copy_from_slice(b"nan"); return 3; }
    if val.is_infinite() {
        if val > 0.0 { tmp[..4].copy_from_slice(b"+inf"); return 4; }
        else { tmp[..4].copy_from_slice(b"-inf"); return 4; }
    }
    let mut pos = 0usize;
    let mut v = val;
    if v < 0.0 { tmp[pos] = b'-'; pos += 1; v = -v; }
    let int_part = v as u64;
    let frac_raw = v - int_part as f64;
    let mut ibuf = [0u8; 24];
    let ilen = fmt_uint(&mut ibuf, int_part, 10, false);
    tmp[pos..pos + ilen].copy_from_slice(&ibuf[..ilen]);
    pos += ilen;
    if prec > 0 {
        tmp[pos] = b'.'; pos += 1;
        let mut frac = frac_raw;
        for _ in 0..prec {
            frac *= 10.0;
            let d = frac as u8;
            tmp[pos] = b'0' + d;
            pos += 1;
            frac -= d as f64;
        }
    }
    pos
}

/// Apply C integer precision (minimum digit count) by inserting leading zeros
/// after an optional leading '-' sign. `digits` is the already-formatted number
/// (with sign for negatives). Returns the slice length written into `dst`.
///
/// Per C99 §7.19.6.1: for d/i/o/u/x/X, precision is the minimum number of
/// digits; shorter values are zero-padded. When precision is specified the `0`
/// flag is ignored (caller must pass zero=false to emit_padded).
fn apply_int_prec(dst: &mut [u8; 64], digits: &[u8], prec: Option<usize>) -> usize {
    let p = match prec {
        Some(p) => p,
        None => {
            let n = digits.len().min(dst.len());
            dst[..n].copy_from_slice(&digits[..n]);
            return n;
        }
    };
    let (sign, num): (&[u8], &[u8]) = if digits.first() == Some(&b'-') {
        (&digits[..1], &digits[1..])
    } else {
        (&[], digits)
    };
    let zeros = p.saturating_sub(num.len());
    let mut i = 0usize;
    for &b in sign { if i < dst.len() { dst[i] = b; } i += 1; }
    for _ in 0..zeros { if i < dst.len() { dst[i] = b'0'; } i += 1; }
    for &b in num { if i < dst.len() { dst[i] = b; } i += 1; }
    i.min(dst.len())
}

// ---------------------------------------------------------------------------
// Padding helper
// ---------------------------------------------------------------------------

pub(super) unsafe fn emit_padded(
    out: *mut u8, pos: &mut usize, cap: usize,
    src: &[u8], width: usize, left: bool, zero: bool,
) {
    let pad_char = if zero && !left { b'0' } else { b' ' };
    let n = src.len();
    let padding = width.saturating_sub(n);
    if !left { for _ in 0..padding { emit_byte(out, pos, cap, pad_char); } }
    for &b in src { emit_byte(out, pos, cap, b); }
    if left  { for _ in 0..padding { emit_byte(out, pos, cap, b' ');     } }
}

#[inline]
pub(super) unsafe fn emit_byte(out: *mut u8, pos: &mut usize, cap: usize, b: u8) {
    if *pos < cap { *out.add(*pos) = b; }
    *pos += 1;
}

// ---------------------------------------------------------------------------
// Core formatter
// ---------------------------------------------------------------------------

/// Format `fmt` with `args` into `out[..size]`; NUL-terminate; return bytes written (excl. NUL).
pub(super) unsafe fn vsnprintf_core(
    out: *mut u8, size: usize, fmt: *const u8, mut args: VaList<'_>,
) -> usize {
    let cap = size.saturating_sub(1);
    let mut pos = 0usize;
    let mut fi = 0usize;

    loop {
        let c = *fmt.add(fi); fi += 1;
        if c == 0 { break; }
        if c != b'%' { emit_byte(out, &mut pos, cap, c); continue; }

        // Flags
        let mut left = false;
        let mut zero = false;
        loop {
            match *fmt.add(fi) {
                b'-' => { left = true; fi += 1; }
                b'0' => { zero = true; fi += 1; }
                _ => break,
            }
        }
        // Width
        let mut width = 0usize;
        while *fmt.add(fi) >= b'0' && *fmt.add(fi) <= b'9' {
            width = width * 10 + (*fmt.add(fi) - b'0') as usize; fi += 1;
        }
        // Precision
        let mut prec: Option<usize> = None;
        if *fmt.add(fi) == b'.' {
            fi += 1;
            let mut p = 0usize;
            while *fmt.add(fi) >= b'0' && *fmt.add(fi) <= b'9' {
                p = p * 10 + (*fmt.add(fi) - b'0') as usize; fi += 1;
            }
            prec = Some(p);
        }
        // Length modifier
        let mut long_mod = 0u8;
        match *fmt.add(fi) {
            b'l' => { fi += 1; long_mod = 1; if *fmt.add(fi) == b'l' { fi += 1; long_mod = 2; } }
            b'h' => { fi += 1; if *fmt.add(fi) == b'h' { fi += 1; } }
            b'z' | b't' => { fi += 1; long_mod = 1; }
            _ => {}
        }
        let spec = *fmt.add(fi); fi += 1;

        match spec {
            b'%' => emit_byte(out, &mut pos, cap, b'%'),
            b'c' => {
                let b = [args.next_arg::<i32>() as u8];
                emit_padded(out, &mut pos, cap, &b, width, left, false);
            }
            b's' => {
                let s = args.next_arg::<*const u8>();
                let raw: &[u8] = if s.is_null() {
                    b"(null)"
                } else {
                    let mut l = 0;
                    while *s.add(l) != 0 { l += 1; }
                    let l = prec.map(|p| p.min(l)).unwrap_or(l);
                    core::slice::from_raw_parts(s, l)
                };
                emit_padded(out, &mut pos, cap, raw, width, left, false);
            }
            b'd' | b'i' => {
                let v: i64 = if long_mod == 2 { args.next_arg::<i64>() }
                             else { args.next_arg::<i32>() as i64 };
                let mut tmp = [0u8; 24]; let l = fmt_int(&mut tmp, v);
                let mut pb = [0u8; 64]; let pl = apply_int_prec(&mut pb, &tmp[..l], prec);
                emit_padded(out, &mut pos, cap, &pb[..pl], width, left, zero && prec.is_none());
            }
            b'u' => {
                let v: u64 = if long_mod == 2 { args.next_arg::<u64>() }
                             else { args.next_arg::<u32>() as u64 };
                let mut tmp = [0u8; 24]; let l = fmt_uint(&mut tmp, v, 10, false);
                let mut pb = [0u8; 64]; let pl = apply_int_prec(&mut pb, &tmp[..l], prec);
                emit_padded(out, &mut pos, cap, &pb[..pl], width, left, zero && prec.is_none());
            }
            b'x' => {
                let v: u64 = if long_mod >= 1 { args.next_arg::<u64>() }
                             else { args.next_arg::<u32>() as u64 };
                let mut tmp = [0u8; 24]; let l = fmt_uint(&mut tmp, v, 16, false);
                let mut pb = [0u8; 64]; let pl = apply_int_prec(&mut pb, &tmp[..l], prec);
                emit_padded(out, &mut pos, cap, &pb[..pl], width, left, zero && prec.is_none());
            }
            b'X' => {
                let v: u64 = if long_mod >= 1 { args.next_arg::<u64>() }
                             else { args.next_arg::<u32>() as u64 };
                let mut tmp = [0u8; 24]; let l = fmt_uint(&mut tmp, v, 16, true);
                let mut pb = [0u8; 64]; let pl = apply_int_prec(&mut pb, &tmp[..l], prec);
                emit_padded(out, &mut pos, cap, &pb[..pl], width, left, zero && prec.is_none());
            }
            b'o' => {
                let v: u64 = if long_mod >= 1 { args.next_arg::<u64>() }
                             else { args.next_arg::<u32>() as u64 };
                let mut tmp = [0u8; 24]; let l = fmt_uint(&mut tmp, v, 8, false);
                let mut pb = [0u8; 64]; let pl = apply_int_prec(&mut pb, &tmp[..l], prec);
                emit_padded(out, &mut pos, cap, &pb[..pl], width, left, zero && prec.is_none());
            }
            b'p' => {
                let v = args.next_arg::<usize>() as u64;
                emit_byte(out, &mut pos, cap, b'0');
                emit_byte(out, &mut pos, cap, b'x');
                let mut tmp = [0u8; 24]; let l = fmt_uint(&mut tmp, v, 16, false);
                emit_padded(out, &mut pos, cap, &tmp[..l], width, left, false);
            }
            b'f' | b'e' | b'g' | b'E' | b'G' => {
                let v = args.next_arg::<f64>();
                let p = prec.unwrap_or(6);
                let mut tmp = [0u8; 64]; let l = fmt_float(&mut tmp, v, p);
                emit_padded(out, &mut pos, cap, &tmp[..l], width, left, zero);
            }
            _ => {
                emit_byte(out, &mut pos, cap, b'%');
                emit_byte(out, &mut pos, cap, spec);
            }
        }
    }

    if size > 0 { *out.add(pos.min(cap)) = 0; }
    pos
}
