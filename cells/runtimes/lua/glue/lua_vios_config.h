/* ViOS Lua 5.4 portable configuration.
 *
 * Included by all Lua C files via -I glue/ (overrides luaconf.h defaults
 * where the defaults require POSIX features we have not fully implemented).
 *
 * This header is safe to include in freestanding C89/C99 contexts.
 */
#pragma once

/* Use the C89-compatible code paths inside Lua. */
#define LUA_USE_C89

/* Route Lua's string I/O through our serial shim.
 * vios_write is defined in lua_vios_glue.c. */
void vios_write(const char *s, size_t n);
#define lua_writestring(s, l)       vios_write((s), (size_t)(l))
#define lua_writeline()             vios_write("\n", 1)
#define lua_writestringerror(s, p)  vios_write((s), __builtin_strlen(s))
