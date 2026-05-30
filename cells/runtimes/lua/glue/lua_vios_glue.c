
/* ViOS Lua 5.4 glue layer.
 *
 * Provides the bare minimum that Lua needs beyond pure C89:
 *   - vios_write: serial output via the POSIX shim _write syscall
 *   - abort:       terminate the current cell
 *   - system/getenv/tmpnam/tmpfile: safe stubs returning failure
 *
 * This file is compiled with the same cross-toolchain as the Lua C sources;
 * it must not include any newlib-specific headers (e.g. sys/reent.h).
 */

#include <stddef.h>   /* size_t, NULL */

/* -- Output ---------------------------------------------------------------- */

/* Write 'n' bytes from 's' to stdout (fd 1) via the POSIX shim.
 * Declared extern so Lua's config header can define lua_writestring etc. */
extern int _write(int fd, const void *buf, unsigned int count);

void vios_write(const char *s, size_t n) {
    if (s && n > 0) {
        _write(1, s, (unsigned int)n);
    }
}

/* -- Process --------------------------------------------------------------- */

extern void _exit(int status);

void abort(void) {
    _exit(1);
    /* unreachable */
    for (;;) {}
}

/* -- Heap (sbrk) ----------------------------------------------------------- */

/* picolibc's malloc family (the public `malloc` AND the reentrant `_malloc_r`
 * that printf's float-formatting/dtoa path calls directly) ultimately grows
 * the heap through `_sbrk`. The toolchain's `_sbrk` is a nosys stub that
 * returns 0, so the first allocation writes a chunk header through null+8 and
 * faults. Overriding `malloc`/`_malloc_r` by symbol definition does NOT work
 * here: the build links with `--allow-multiple-definition`, under which the
 * archive's copy wins by link order.
 *
 * The robust fix is a linker `--wrap`: every reference to `_sbrk` (including
 * those inside libc's own `_sbrk_r`) is rewritten to `__wrap__sbrk`, so this
 * implementation is used regardless of link order. It hands out a static heap
 * arena in the cell's BSS (zeroed by the ELF loader). Single-threaded cell, so
 * no locking. With a real heap, picolibc's own malloc/realloc/free run
 * unmodified — no allocator reimplementation needed. */
#include <stdint.h>
#include <stddef.h>

#define VIOS_HEAP_BYTES (8 * 1024 * 1024)
static unsigned char vios_heap[VIOS_HEAP_BYTES];
static size_t vios_heap_used = 0;

/* sbrk contract: grow the break by `incr` bytes and return the PREVIOUS break
 * (the start of the newly granted region); return (void*)-1 on exhaustion.
 * `incr` may be negative (shrink) or zero (query current break). */
void *__wrap__sbrk(ptrdiff_t incr) {
    size_t prev = vios_heap_used;
    if (incr < 0) {
        size_t dec = (size_t)(-incr);
        vios_heap_used = (dec > prev) ? 0 : prev - dec;
        return &vios_heap[vios_heap_used];
    }
    if (prev + (size_t)incr > VIOS_HEAP_BYTES) {
        return (void *)-1; /* out of heap */
    }
    vios_heap_used = prev + (size_t)incr;
    return &vios_heap[prev];
}

/* -- Stub POSIX helpers Lua may call --------------------------------------- */

int system(const char *cmd) { (void)cmd; return -1; }

char *getenv(const char *name) { (void)name; return (char *)0; }

/* tmpnam / tmpfile: Lua's io lib may call these; return failure. */
char *tmpnam(char *s) { (void)s; return (char *)0; }

/* FILE is defined in lua's own include path; avoid pulling in stdio.h */
void *tmpfile(void) { return (void *)0; }
