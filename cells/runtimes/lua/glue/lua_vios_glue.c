
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

/* -- Stub POSIX helpers Lua may call --------------------------------------- */

int system(const char *cmd) { (void)cmd; return -1; }

char *getenv(const char *name) { (void)name; return (char *)0; }

/* tmpnam / tmpfile: Lua's io lib may call these; return failure. */
char *tmpnam(char *s) { (void)s; return (char *)0; }

/* FILE is defined in lua's own include path; avoid pulling in stdio.h */
void *tmpfile(void) { return (void *)0; }
