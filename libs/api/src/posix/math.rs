// SPDX-License-Identifier: MPL-2.0
// C99 math bridge — exposes Rust `libm` functions as `#[no_mangle] extern "C"` symbols.
//
// Cells linking with the `posix` feature must NOT pass `-lm` to the linker;
// this module owns all C99 math symbols for riscv64, aarch64, and wasm32.

#![allow(unsafe_code)]

// ---------------------------------------------------------------------------
// Helper macros — no #[inline] on #[no_mangle] (compiler ignores it anyway)
// ---------------------------------------------------------------------------

macro_rules! math1_f64 {
    ($c:ident, $rust:path) => {
        #[no_mangle] pub extern "C" fn $c(x: f64) -> f64 { $rust(x) }
    };
}

macro_rules! math2_f64 {
    ($c:ident, $rust:path) => {
        #[no_mangle] pub extern "C" fn $c(x: f64, y: f64) -> f64 { $rust(x, y) }
    };
}

macro_rules! math1_f32 {
    ($c:ident, $rust:path) => {
        #[no_mangle] pub extern "C" fn $c(x: f32) -> f32 { $rust(x) }
    };
}

macro_rules! math2_f32 {
    ($c:ident, $rust:path) => {
        #[no_mangle] pub extern "C" fn $c(x: f32, y: f32) -> f32 { $rust(x, y) }
    };
}

// ---------------------------------------------------------------------------
// Double (f64) — one-argument functions
// ---------------------------------------------------------------------------

math1_f64!(sin,   libm::sin);
math1_f64!(cos,   libm::cos);
math1_f64!(tan,   libm::tan);
math1_f64!(asin,  libm::asin);
math1_f64!(acos,  libm::acos);
math1_f64!(atan,  libm::atan);
math1_f64!(sinh,  libm::sinh);
math1_f64!(cosh,  libm::cosh);
math1_f64!(tanh,  libm::tanh);
math1_f64!(asinh, libm::asinh);
math1_f64!(acosh, libm::acosh);
math1_f64!(atanh, libm::atanh);
math1_f64!(exp,   libm::exp);
math1_f64!(exp2,  libm::exp2);
math1_f64!(expm1, libm::expm1);
math1_f64!(log,   libm::log);
math1_f64!(log2,  libm::log2);
math1_f64!(log10, libm::log10);
math1_f64!(log1p, libm::log1p);
math1_f64!(sqrt,  libm::sqrt);
math1_f64!(cbrt,  libm::cbrt);
math1_f64!(floor, libm::floor);
math1_f64!(ceil,  libm::ceil);
math1_f64!(round, libm::round);
math1_f64!(trunc, libm::trunc);
math1_f64!(rint,  libm::rint);
math1_f64!(fabs,  libm::fabs);

// nearbyint — equivalent to rint in bare-metal (no FE_INEXACT trap)
#[no_mangle] pub extern "C" fn nearbyint(x: f64) -> f64 { libm::rint(x) }

// logb — returns unbiased exponent as f64; delegate to ilogb
#[no_mangle] pub extern "C" fn logb(x: f64) -> f64 { libm::ilogb(x) as f64 }

// ---------------------------------------------------------------------------
// Double — two-argument functions
// ---------------------------------------------------------------------------

math2_f64!(atan2,     libm::atan2);
math2_f64!(pow,       libm::pow);
math2_f64!(hypot,     libm::hypot);
math2_f64!(fmod,      libm::fmod);
math2_f64!(remainder, libm::remainder);
math2_f64!(fmin,      libm::fmin);
math2_f64!(fmax,      libm::fmax);
math2_f64!(copysign,  libm::copysign);
math2_f64!(nextafter, libm::nextafter);

// ---------------------------------------------------------------------------
// Double — special signatures
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn fma(x: f64, y: f64, z: f64) -> f64 { libm::fma(x, y, z) }

#[no_mangle]
pub extern "C" fn scalbn(x: f64, n: i32) -> f64 { libm::scalbn(x, n) }

#[no_mangle]
pub extern "C" fn ldexp(x: f64, n: i32) -> f64 { libm::ldexp(x, n) }

#[no_mangle]
pub extern "C" fn ilogb(x: f64) -> i32 { libm::ilogb(x) }

/// modf: splits x into integer and fractional parts; stores integer in *iptr.
#[no_mangle]
pub unsafe extern "C" fn modf(x: f64, iptr: *mut f64) -> f64 {
    let (frac, int_part) = libm::modf(x);
    if !iptr.is_null() { *iptr = int_part; }
    frac
}

/// frexp: decomposes x into mantissa ∈ [0.5, 1) and exponent.
#[no_mangle]
pub unsafe extern "C" fn frexp(x: f64, exp: *mut i32) -> f64 {
    let (mantissa, exponent) = libm::frexp(x);
    if !exp.is_null() { *exp = exponent; }
    mantissa
}

// ---------------------------------------------------------------------------
// Float (f32) — one-argument functions
// ---------------------------------------------------------------------------

math1_f32!(sinf,   libm::sinf);
math1_f32!(cosf,   libm::cosf);
math1_f32!(tanf,   libm::tanf);
math1_f32!(asinf,  libm::asinf);
math1_f32!(acosf,  libm::acosf);
math1_f32!(atanf,  libm::atanf);
math1_f32!(sinhf,  libm::sinhf);
math1_f32!(coshf,  libm::coshf);
math1_f32!(tanhf,  libm::tanhf);
math1_f32!(asinhf, libm::asinhf);
math1_f32!(acoshf, libm::acoshf);
math1_f32!(atanhf, libm::atanhf);
math1_f32!(expf,   libm::expf);
math1_f32!(exp2f,  libm::exp2f);
math1_f32!(expm1f, libm::expm1f);
math1_f32!(logf,   libm::logf);
math1_f32!(log2f,  libm::log2f);
math1_f32!(log10f, libm::log10f);
math1_f32!(log1pf, libm::log1pf);
math1_f32!(sqrtf,  libm::sqrtf);
math1_f32!(cbrtf,  libm::cbrtf);
math1_f32!(floorf, libm::floorf);
math1_f32!(ceilf,  libm::ceilf);
math1_f32!(roundf, libm::roundf);
math1_f32!(truncf, libm::truncf);
math1_f32!(rintf,  libm::rintf);
math1_f32!(fabsf,  libm::fabsf);

#[no_mangle] pub extern "C" fn nearbyintf(x: f32) -> f32 { libm::rintf(x) }
#[no_mangle] pub extern "C" fn logbf(x: f32) -> f32 { libm::ilogbf(x) as f32 }

// ---------------------------------------------------------------------------
// Float — two-argument functions
// ---------------------------------------------------------------------------

math2_f32!(atan2f,     libm::atan2f);
math2_f32!(powf,       libm::powf);
math2_f32!(hypotf,     libm::hypotf);
math2_f32!(fmodf,      libm::fmodf);
math2_f32!(remainderf, libm::remainderf);
math2_f32!(fminf,      libm::fminf);
math2_f32!(fmaxf,      libm::fmaxf);
math2_f32!(copysignf,  libm::copysignf);
math2_f32!(nextafterf, libm::nextafterf);

// ---------------------------------------------------------------------------
// Float — special signatures
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn fmaf(x: f32, y: f32, z: f32) -> f32 { libm::fmaf(x, y, z) }

#[no_mangle]
pub extern "C" fn scalbnf(x: f32, n: i32) -> f32 { libm::scalbnf(x, n) }

#[no_mangle]
pub extern "C" fn ldexpf(x: f32, n: i32) -> f32 { libm::ldexpf(x, n) }

#[no_mangle]
pub extern "C" fn ilogbf(x: f32) -> i32 { libm::ilogbf(x) }

/// modff: float variant of modf.
#[no_mangle]
pub unsafe extern "C" fn modff(x: f32, iptr: *mut f32) -> f32 {
    let (frac, int_part) = libm::modff(x);
    if !iptr.is_null() { *iptr = int_part; }
    frac
}

/// frexpf: float variant of frexp.
#[no_mangle]
pub unsafe extern "C" fn frexpf(x: f32, exp: *mut i32) -> f32 {
    let (mantissa, exponent) = libm::frexpf(x);
    if !exp.is_null() { *exp = exponent; }
    mantissa
}
