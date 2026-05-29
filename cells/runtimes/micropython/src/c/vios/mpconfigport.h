/* ViOS MicroPython port configuration.
 *
 * Targets RISC-V 64 bare-metal (no POSIX, no threads, no filesystem yet).
 * I/O routes through OSTD console HAL hooks defined in mphalport.c.
 */

#ifndef MPCONFIGPORT_H
#define MPCONFIGPORT_H

#include <stdint.h>
#include <stddef.h>
#include <alloca.h>

/* ── Integer types (64-bit RISC-V) ─────────────────────────────────────── */
typedef intptr_t  mp_int_t;
typedef uintptr_t mp_uint_t;
typedef long      mp_off_t;

#define MP_SSIZE_MAX   ((mp_int_t)(((mp_uint_t)-1) >> 1))
#define BYTES_PER_WORD ((int)sizeof(mp_int_t))

/* ── Object representation ──────────────────────────────────────────────── */
/* Tagged-pointer (REPR_A): integers encoded as (n<<1)|1, objects as pointers.
 * Simpler than NaN-boxing; floats require a heap allocation but that is fine
 * for a scripting REPL.  REPR_D caused type-mismatch errors in objint.c. */
#define MICROPY_OBJ_REPR           MICROPY_OBJ_REPR_A

/* ── Integer implementation ─────────────────────────────────────────────── */
/* long long avoids mpz.c arbitrary-precision dependency */
#define MICROPY_LONGINT_IMPL       MICROPY_LONGINT_IMPL_LONGLONG

/* ── Float ──────────────────────────────────────────────────────────────── */
#define MICROPY_FLOAT_IMPL         MICROPY_FLOAT_IMPL_DOUBLE

/* ── NLR (non-local return / exceptions via setjmp) ─────────────────────── */
#define MICROPY_NLR_SETJMP         1

/* ── Core features ──────────────────────────────────────────────────────── */
#define MICROPY_ENABLE_COMPILER    1
#define MICROPY_ENABLE_GC          1
#define MICROPY_HELPER_REPL        1
#define MICROPY_REPL_EVENT_DRIVEN  0
#define MICROPY_ENABLE_SOURCE_LINE 1
#define MICROPY_STACK_CHECK        0

/* ── Standard library modules ───────────────────────────────────────────── */
#define MICROPY_PY_BUILTINS_BYTEARRAY  1
#define MICROPY_PY_BUILTINS_MEMORYVIEW 1
#define MICROPY_PY_BUILTINS_FROZENSET  1
#define MICROPY_PY_BUILTINS_STR_UNICODE 1
#define MICROPY_PY_COLLECTIONS     1
#define MICROPY_PY_MATH            1
#define MICROPY_PY_GC              1
#define MICROPY_PY_SYS             1
#define MICROPY_PY_SYS_EXIT        1
#define MICROPY_PY_STRUCT          1
#define MICROPY_PY_ERRNO           0  /* avoids MP_QSTR_EPERM etc. not in static QSTR pool */
/* Defer until VFS IPC is wired */
#define MICROPY_PY_IO              0
#define MICROPY_PY_OS              0
/* No OS-level threads in ViOS LBI cells */
#define MICROPY_PY_THREAD          0

/* ── Memory / debug ─────────────────────────────────────────────────────── */
#define MICROPY_MEM_STATS          0
#define MICROPY_DEBUG_PRINTERS     0
/* mp_printf routes to mp_hal_stdout_tx_strn in mphalport.c.
 * mp_hal_stdout_tx_str is a real function (not macro) defined in mphalport.c
 * so the linker always finds it regardless of inclusion order. */
#define MICROPY_USE_INTERNAL_PRINTF 1

/* ── Scheduler / VFS (disabled) ─────────────────────────────────────────── */
#define MICROPY_ENABLE_SCHEDULER   0
/* Must define MICROPY_SCHEDULER_DEPTH even when scheduler is off;
 * root_pointers.h declares sched_queue[MICROPY_SCHEDULER_DEPTH] unconditionally. */
#define MICROPY_SCHEDULER_DEPTH    4
#define MICROPY_READER_VFS         0
#define MICROPY_VFS                0

/* ── Platform identification ────────────────────────────────────────────── */
#define MICROPY_PY_SYS_PLATFORM      "vios"
#define MICROPY_PLATFORM_COMPILER    "riscv-none-elf-gcc"
#define MICROPY_BANNER_MACHINE       "ViOS (Cellular SAS)"

/* ── Port hooks (no-ops for now) ────────────────────────────────────────── */
#define MICROPY_PORT_INIT_FUNC
#define MICROPY_PORT_DEINIT_FUNC
#define MICROPY_PORT_ROOT_POINTERS
#define MICROPY_PORT_BUILTIN_MODULES

#endif /* MPCONFIGPORT_H */
