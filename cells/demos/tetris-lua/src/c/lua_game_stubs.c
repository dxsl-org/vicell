/* Stub implementations for Lua standard libraries not needed by the Tetris game.
 *
 * loslib.c pulls in strftime/localtime from picolibc, which contains
 * non-PIC (R_RISCV_64) relocations in _C_time_locale — incompatible with
 * PIE cell linking.  liolib.c similarly pulls in FILE* stdio machinery.
 *
 * The tetris.lua script uses only the Rust-provided `surface`, `input`, and
 * `time` tables — it never calls os.* or io.*.  These stubs keep linit.c's
 * registration table intact while avoiding the problematic C runtime symbols.
 */

/* Forward-declare only what linit.c needs — avoids including lua.h here. */
typedef struct lua_State lua_State;
typedef int (*lua_CFunction)(lua_State *L);

/* lua.h extern C guard not needed (this file is compiled as C) */
extern void lua_createtable(lua_State *L, int narr, int nrec);

int luaopen_io(lua_State *L) {
    lua_createtable(L, 0, 0);   /* push empty table — io.* not available */
    return 1;
}

int luaopen_os(lua_State *L) {
    lua_createtable(L, 0, 0);   /* push empty table — os.* not available */
    return 1;
}
