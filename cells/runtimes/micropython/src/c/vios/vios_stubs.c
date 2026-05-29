/* ViOS MicroPython stubs.
 *
 * Provides no-op or minimal stubs for MicroPython symbols that are referenced
 * by the core runtime but not yet wired in ViOS (VFS, readline, disabled modules).
 */

#include <stddef.h>
#include <string.h>
#include "py/mpconfig.h"
#include "py/misc.h"
#include "py/mphal.h"
#include "py/lexer.h"
#include "py/runtime.h"
#include "py/obj.h"
#include "py/objmodule.h"
#include "py/builtin.h"

/* ── readline ───────────────────────────────────────────────────────────── */
/* Minimal blocking readline used by pyexec_friendly_repl().
 * readline.h declares: void readline_init0(void); int readline(vstr_t*, const char*);
 * Returns: 0 = normal line, CHAR_CTRL_C = interrupt, CHAR_CTRL_D = EOF. */
void readline_init0(void) {}

int readline(vstr_t *line, const char *prompt) {
    mp_hal_stdout_tx_str(prompt);
    vstr_reset(line);
    for (;;) {
        int c = mp_hal_stdin_rx_chr();
        if (c == '\r' || c == '\n') {
            mp_hal_stdout_tx_strn("\r\n", 2);
            return 0; /* normal line */
        }
        if (c == 4) { /* Ctrl-D = EOF */
            return 4;
        }
        if (c == 3) { /* Ctrl-C = interrupt */
            mp_hal_stdout_tx_strn("\r\n", 2);
            return 3;
        }
        if ((c == 127 || c == 8) && line->len > 0) { /* Backspace */
            vstr_cut_tail_bytes(line, 1);
            mp_hal_stdout_tx_strn("\b \b", 3);
        } else if (c >= 32 && c < 127) {
            vstr_add_byte(line, (byte)c);
            mp_hal_stdout_tx_strn((const char *)&c, 1);
        }
    }
}

void readline_push_history(const char *line) { (void)line; }
void readline_init(vstr_t *line, const char *prompt) { (void)line; (void)prompt; }
void readline_note_newline(const char *prompt) { (void)prompt; }
int  readline_process_char(int c) { return c; }

/* ── mp_lexer_new_from_file ─────────────────────────────────────────────── */
/* Called when Python tries to `import` a .py file. No VFS wired yet. */
mp_lexer_t *mp_lexer_new_from_file(qstr filename) {
    (void)filename;
    mp_raise_OSError(1); /* EPERM */
    return NULL; /* unreachable */
}

/* ── mp_import_stat ─────────────────────────────────────────────────────── */
/* Called to check if an importable module exists. Always not-found. */
mp_import_stat_t mp_import_stat(const char *path) {
    (void)path;
    return MP_IMPORT_STAT_NO_EXIST;
}

/* ── Disabled module objects ─────────────────────────────────────────────── */
/* moduledefs.h emits extern declarations for every scanned MP_REGISTER_MODULE
 * call even when a feature flag disables that module.  Provide minimal module
 * objects that satisfy the linker without doing anything. */

#if !MICROPY_PY_THREAD
const mp_obj_module_t mp_module_thread = {
    .base    = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&mp_module_builtins.globals,
};
#endif

#if !MICROPY_PY_ERRNO
const mp_obj_module_t mp_module_errno = {
    .base    = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&mp_module_builtins.globals,
};
#endif

#if !MICROPY_PY_IO
const mp_obj_module_t mp_module_io = {
    .base    = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&mp_module_builtins.globals,
};
#endif

/* ── mp_hal_set_interrupt_char ──────────────────────────────────────────── */
/* interrupt_char.c only defines this when MICROPY_KBD_EXCEPTION is set.
 * Provide a no-op so pyexec can call it without the feature enabled. */
#if !MICROPY_KBD_EXCEPTION
void mp_hal_set_interrupt_char(int c) { (void)c; }
#endif

/* ── mp_module_cmath ────────────────────────────────────────────────────── */
/* modcmath.c guards the module behind MICROPY_PY_CMATH + COMPLEX + FLOAT.
 * Provide an empty module when any of those flags is off. */
#if !(MICROPY_PY_BUILTINS_FLOAT && MICROPY_PY_BUILTINS_COMPLEX && MICROPY_PY_CMATH)
const mp_obj_module_t mp_module_cmath = {
    .base    = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&mp_module_builtins.globals,
};
#endif
