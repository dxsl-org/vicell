#![no_std]
#![no_main]
extern crate ostd;

/// Convert Unix epoch seconds to (year, month, day, hour, min, sec) in UTC.
fn epoch_to_datetime(mut secs: u64) -> (u64, u8, u8, u8, u8, u8) {
    fn is_leap(y: u64) -> bool {
        (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
    }
    fn days_in_month(m: u8, y: u64) -> u64 {
        match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11              => 30,
            2                           => if is_leap(y) { 29 } else { 28 },
            _                           => 0,
        }
    }
    let mut year = 1970u64;
    loop {
        let year_secs = if is_leap(year) { 366 * 86400 } else { 365 * 86400 };
        if secs < year_secs { break; }
        secs -= year_secs;
        year += 1;
    }
    let mut month = 1u8;
    loop {
        let m_secs = days_in_month(month, year) * 86400;
        if secs < m_secs { break; }
        secs -= m_secs;
        month += 1;
    }
    let day  = (secs / 86400 + 1) as u8;
    secs    %= 86400;
    let hour = (secs / 3600) as u8;
    secs    %= 3600;
    let min  = (secs / 60) as u8;
    let sec  = (secs % 60) as u8;
    (year, month, day, hour, min, sec)
}

fn print_pad2(n: u8) {
    if n < 10 { ostd::io::print("0"); }
    ostd::io::print_usize(n as usize);
}

fn print_pad4(n: u64) {
    if      n < 10   { ostd::io::print("000"); }
    else if n < 100  { ostd::io::print("00"); }
    else if n < 1000 { ostd::io::print("0"); }
    ostd::io::print_usize(n as usize);
}

/// date — print wall-clock time from hardware RTC (UTC).
#[no_mangle]
pub fn main() {
    let epoch = ostd::syscall::sys_get_wall_secs();
    if epoch == 0 {
        // RTC absent — fall back to uptime display.
        let ticks = ostd::syscall::sys_get_time();
        let secs  = ticks / 10_000_000; // 10 MHz mtime
        let mins  = secs / 60;
        let hrs   = mins / 60;
        ostd::io::print("Uptime: ");
        ostd::io::print_usize(hrs as usize);
        ostd::io::print("h ");
        ostd::io::print_usize((mins % 60) as usize);
        ostd::io::print("m ");
        ostd::io::print_usize((secs % 60) as usize);
        ostd::io::println("s  (no RTC)");
    } else {
        let (y, mo, d, h, mi, s) = epoch_to_datetime(epoch);
        // Format: 2026-06-07 15:30:42 UTC
        print_pad4(y);
        ostd::io::print("-");
        print_pad2(mo);
        ostd::io::print("-");
        print_pad2(d);
        ostd::io::print(" ");
        print_pad2(h);
        ostd::io::print(":");
        print_pad2(mi);
        ostd::io::print(":");
        print_pad2(s);
        ostd::io::println(" UTC");
    }
    ostd::syscall::sys_exit(0);
}
