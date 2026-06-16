// SPDX-License-Identifier: MPL-2.0

use crate::*;
use alloc::string::String;

// ─── embedded-io glue ────────────────────────────────────────────────────────

/// Opaque I/O error wrapping a [`ViError`] for [`embedded_io`] trait impls.
///
/// `ViError` lives in `libs/types`; implementing a foreign trait on a foreign type
/// violates the orphan rule. This newtype lives in ostd and bridges the two.
#[derive(Debug)]
pub struct OstdError(pub ViError);

impl core::fmt::Display for OstdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl core::error::Error for OstdError {}

impl embedded_io::Error for OstdError {
    fn kind(&self) -> embedded_io::ErrorKind {
        match self.0 {
            ViError::NotFound         => embedded_io::ErrorKind::NotFound,
            ViError::PermissionDenied => embedded_io::ErrorKind::PermissionDenied,
            ViError::OutOfMemory      => embedded_io::ErrorKind::OutOfMemory,
            ViError::WouldBlock       => embedded_io::ErrorKind::Other,
            _                         => embedded_io::ErrorKind::Other,
        }
    }
}

/// Print to console.
pub fn print(s: &str) {
    let _ = syscall::sys_log(s);
}

/// Print line to console.
pub fn println(s: &str) {
    print(s);
    print("\n");
}

pub fn print_usize(n: usize) {
    let mut buf = [0u8; 20];
    let mut i = 0;
    let mut num = n;
    
    if num == 0 {
        print("0");
        return;
    }
    
    while num > 0 {
        buf[i] = (num % 10) as u8 + b'0';
        num /= 10;
        i += 1;
    }
    
    // Reverse
    let mut start = 0;
    let mut end = i - 1;
    while start < end {
        let tmp = buf[start];
        buf[start] = buf[end];
        buf[end] = tmp;
        start += 1;
        end -= 1;
    }
    
    if let Ok(s) = core::str::from_utf8(&buf[..i]) {
        print(s);
    }
}

pub struct Stdin;

impl Stdin {
    pub fn read_line(&self, buf: &mut String) -> ViResult<usize> {
        let mut bytes_read = 0;
        loop {
            let mut c = [0u8; 1];
            if let Ok(n) = syscall::sys_read(0, &mut c) {
                if n > 0 {
                    let ch = c[0] as char;
                    // Echo is handled by kernel now?
                    // Kernel sys_read implementation I wrote DOES echo.

                    // Echo back
                    if ch == '\r' || ch == '\n' {
                        print("\n");
                        buf.push('\n');
                        return Ok(bytes_read + 1);
                    }

                    // Handle Backspace (127 or 8)
                    if c[0] == 8 || c[0] == 127 {
                        if !buf.is_empty() {
                            // Print backspace sequence to erase char on screen
                            // \x08 (Back) space \x08
                            print("\x08 \x08");
                            buf.pop();
                            bytes_read -= 1;
                        }
                        continue;
                    }

                    // Normal char
                    let mut tmp = [0u8; 4];
                    let s = ch.encode_utf8(&mut tmp);
                    print(s);

                    buf.push(ch);
                    bytes_read += 1;
                } else {
                    // NO BLOCKING? sys_read usually blocks.
                    // But my sys_read implementation loops.
                    // Wait, my sys_read implementation loops with yielding.
                    // So it blocks until input.
                }
            } else {
                return Err(ViError::IO);
            }
        }
    }
}

pub fn stdin() -> Stdin {
    Stdin
}

// ─── embedded-io trait impls ─────────────────────────────────────────────────

impl embedded_io::ErrorType for Stdin {
    type Error = OstdError;
}

impl embedded_io::Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, OstdError> {
        syscall::sys_read(0, buf).map_err(|_| OstdError(ViError::IO))
    }
}

/// Handle to the standard output stream.
pub struct Stdout;

pub fn stdout() -> Stdout {
    Stdout
}

impl embedded_io::ErrorType for Stdout {
    type Error = OstdError;
}

impl embedded_io::Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize, OstdError> {
        match core::str::from_utf8(buf) {
            Ok(s) => {
                syscall::sys_log(s); // always succeeds; return value is ignored
                Ok(buf.len())
            }
            Err(_) => Err(OstdError(ViError::InvalidInput)),
        }
    }

    fn flush(&mut self) -> Result<(), OstdError> {
        Ok(()) // sys_log is synchronous; no buffering to flush
    }
}
