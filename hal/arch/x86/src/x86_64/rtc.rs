//! CMOS RTC driver for x86_64.
//!
//! Reads calendar time from CMOS via I/O ports 0x70 (index) / 0x71 (data).
//! Always available on x86_64 — no init required.
//!
//! Registers (BCD-encoded on QEMU):
//!   0x00=Seconds  0x02=Minutes  0x04=Hours  0x07=Day  0x08=Month
//!   0x09=Year(2dig)  0x0A=StatusA(UIP=bit7)  0x32=Century

unsafe fn outb(val: u8, port: u16) {
    // SAFETY: I/O port write is a Ring-0 privileged instruction; no memory invariants affected.
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags),
    );
}

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: I/O port read from Ring-0; no memory invariants affected.
    core::arch::asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags),
    );
    val
}

fn cmos_read(reg: u8) -> u8 {
    // SAFETY: 0x70/0x71 are the standard CMOS index/data ports on x86; called from Ring-0.
    unsafe {
        outb(reg, 0x70);
        inb(0x71)
    }
}

fn bcd(v: u8) -> u8 {
    (v >> 4) * 10 + (v & 0x0F)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(month: u8, year: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11               => 30,
        2                             => if is_leap(year) { 29 } else { 28 },
        _                             => 0,
    }
}

/// Calendar date/time → Unix epoch seconds.
fn calendar_to_epoch(year: u64, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> u64 {
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += days_in_month(m, year);
    }
    days += day as u64 - 1;
    days * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64
}

/// Nanoseconds since Unix epoch from CMOS RTC (second-level resolution).
///
/// Waits for CMOS Update-In-Progress to clear, then performs a double-read
/// to avoid torn values across a second boundary.
pub fn now_epoch_ns() -> u64 {
    let (sec, min, hour, day, month, year);
    loop {
        // Wait for CMOS update to finish.
        while cmos_read(0x0A) & 0x80 != 0 {
            core::hint::spin_loop();
        }
        let s  = cmos_read(0x00);
        let mi = cmos_read(0x02);
        let h  = cmos_read(0x04);
        let d  = cmos_read(0x07);
        let mo = cmos_read(0x08);
        let y  = cmos_read(0x09);
        let c  = cmos_read(0x32);
        // Double-read: confirm seconds haven't changed.
        while cmos_read(0x0A) & 0x80 != 0 {
            core::hint::spin_loop();
        }
        if s != cmos_read(0x00) {
            continue;
        }
        sec   = bcd(s);
        min   = bcd(mi);
        hour  = bcd(h);
        day   = bcd(d);
        month = bcd(mo);
        // Century register 0x32: treat 0 or implausible values as 21st century.
        let cent = if c == 0 || c > 0x30 { 20u8 } else { bcd(c) };
        year  = bcd(y) as u64 + cent as u64 * 100;
        break;
    }
    calendar_to_epoch(year, month, day, hour, min, sec) * 1_000_000_000
}
