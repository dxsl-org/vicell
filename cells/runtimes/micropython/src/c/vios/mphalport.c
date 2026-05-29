/* ViOS MicroPython HAL — I/O via the POSIX shim (_write / _read syscalls).
 *
 * mp_hal_stdout_tx_strn   — write n bytes to the serial console
 * mp_hal_stdin_rx_chr     — read one character from the serial console
 * mp_hal_delay_ms         — best-effort busy wait (no OS timer in bare-metal)
 * mp_hal_ticks_ms         — returns 0 (no RTC yet)
 * abort                   — halt the cell
 *
 * These two POSIX shims must be provided by the newlib/picolibc layer that
 * ViOS links against for every cell:
 *   extern int _write(int fd, const void *buf, unsigned int count);
 *   extern int _read(int fd, void *buf, unsigned int count);
 */

#include <stddef.h>
#include <stdint.h>
#include "py/mpconfig.h"
#include "py/misc.h"
#include "py/mphal.h"

/* Provided by the ViOS newlib/picolibc POSIX shim. */
extern int _write(int fd, const void *buf, unsigned int count);
extern int _read(int fd, void *buf, unsigned int count);
extern void _exit(int status);

/* ── Output ─────────────────────────────────────────────────────────────── */

mp_uint_t mp_hal_stdout_tx_strn(const char *str, size_t len) {
    if (str && len > 0) {
        _write(1, str, (unsigned int)len);
    }
    return (mp_uint_t)len;
}

/* Single-string output — declared in mphal.h as void, called by pyexec etc. */
void mp_hal_stdout_tx_str(const char *str) {
    if (str) {
        size_t len = 0;
        while (str[len]) len++;
        _write(1, str, (unsigned int)len);
    }
}

/* Cooked variant: convert bare \n → \r\n for serial terminals. */
void mp_hal_stdout_tx_strn_cooked(const char *str, size_t len) {
    const char *end = str + len;
    while (str < end) {
        const char *nl = str;
        while (nl < end && *nl != '\n') {
            nl++;
        }
        if (nl > str) {
            _write(1, str, (unsigned int)(nl - str));
        }
        if (nl < end) {
            _write(1, "\r\n", 2);
            nl++;
        }
        str = nl;
    }
}

/* ── Input ──────────────────────────────────────────────────────────────── */

int mp_hal_stdin_rx_chr(void) {
    unsigned char c = 0;
    _read(0, &c, 1);
    return (int)c;
}

/* ── Timing (stubs — no timer hardware yet) ─────────────────────────────── */

void mp_hal_delay_ms(mp_uint_t ms) {
    /* Bare-metal busy wait: ~1M iterations ≈ 1ms at 1GHz. Very approximate. */
    volatile uint64_t i = (uint64_t)ms * 1000000ULL;
    while (i--) {
        __asm__ volatile("nop");
    }
}

void mp_hal_delay_us(mp_uint_t us) {
    volatile uint64_t i = (uint64_t)us * 1000ULL;
    while (i--) {
        __asm__ volatile("nop");
    }
}

mp_uint_t mp_hal_ticks_ms(void) {
    return 0;
}

mp_uint_t mp_hal_ticks_us(void) {
    return 0;
}

mp_uint_t mp_hal_ticks_cpu(void) {
    return 0;
}

/* ── Process termination ────────────────────────────────────────────────── */

void abort(void) {
    _exit(1);
    /* SAFETY: _exit never returns; this loop silences the noreturn warning. */
    for (;;) {}
}
