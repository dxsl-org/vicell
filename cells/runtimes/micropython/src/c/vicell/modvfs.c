/*
 * ViCell MicroPython `vfs` module — typed IPC edition.
 *
 * All VFS operations now go through the C-callable Rust bridge (vfs_bridge.rs)
 * which speaks the typed postcard VfsRequest/VfsResponse protocol that the VFS
 * cell expects since Milestone 2.1.  The old raw byte-opcode protocol
 * (OP_READ=8, OP_WRITE=4, …) has been removed from both sides.
 *
 * Exposed Python API:
 *   vfs.read(path)              -> str | None
 *   vfs.write(path, content)    -> bool
 *   vfs.append(path, content)   -> bool
 *   vfs.mkdir(path)             -> bool
 *   vfs.stat(path)              -> (size:int, is_dir:bool) | None
 *   vfs.listdir(path)           -> list[str] | None
 *   vfs.remove(path)            -> bool
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>

#include "py/obj.h"
#include "py/runtime.h"
#include "py/objlist.h"

/* ── Typed VFS bridge (vfs_bridge.rs) ────────────────────────────────────── */
extern size_t ViCell_vfs_read   (const uint8_t *path, size_t pl, uint8_t *out, size_t out_size);
extern int    ViCell_vfs_write  (const uint8_t *path, size_t pl, const uint8_t *data, size_t dl);
extern int    ViCell_vfs_append (const uint8_t *path, size_t pl, const uint8_t *data, size_t dl);
extern int    ViCell_vfs_mkdir  (const uint8_t *path, size_t pl);
extern int    ViCell_vfs_stat   (const uint8_t *path, size_t pl, uint64_t *size_out, int *is_dir_out);
extern size_t ViCell_vfs_listdir(const uint8_t *path, size_t pl, uint8_t *out, size_t out_size);
extern int    ViCell_vfs_remove (const uint8_t *path, size_t pl);

/* ── Static buffers (single-threaded cell — safe) ─────────────────────────── */
/* 64 KB read cap (matches the GetFile truncation limit in vfs_bridge.rs); in .bss. */
static uint8_t s_read_buf[64 * 1024];
/* VFS ListDir reply fits in 512 bytes (~30 entries at the VFS protocol limit). */
static uint8_t s_listdir_buf[512];

/* ── vfs.read(path) -> str | None ────────────────────────────────────────── */
static mp_obj_t vfs_read(mp_obj_t path_obj) {
    size_t pl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    size_t n = ViCell_vfs_read((const uint8_t *)path, pl, s_read_buf, sizeof(s_read_buf));
    if (n == 0) return mp_const_none;
    return mp_obj_new_str((const char *)s_read_buf, n);
}
static MP_DEFINE_CONST_FUN_OBJ_1(vfs_read_obj, vfs_read);

/* ── vfs.write(path, content) -> bool ───────────────────────────────────── */
static mp_obj_t vfs_write(mp_obj_t path_obj, mp_obj_t data_obj) {
    size_t pl, dl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    const char *data = mp_obj_str_get_data(data_obj, &dl);
    return ViCell_vfs_write((const uint8_t *)path, pl, (const uint8_t *)data, dl)
           ? mp_const_true : mp_const_false;
}
static MP_DEFINE_CONST_FUN_OBJ_2(vfs_write_obj, vfs_write);

/* ── vfs.append(path, content) -> bool ──────────────────────────────────── */
static mp_obj_t vfs_append(mp_obj_t path_obj, mp_obj_t data_obj) {
    size_t pl, dl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    const char *data = mp_obj_str_get_data(data_obj, &dl);
    return ViCell_vfs_append((const uint8_t *)path, pl, (const uint8_t *)data, dl)
           ? mp_const_true : mp_const_false;
}
static MP_DEFINE_CONST_FUN_OBJ_2(vfs_append_obj, vfs_append);

/* ── vfs.mkdir(path) -> bool ─────────────────────────────────────────────── */
static mp_obj_t vfs_mkdir(mp_obj_t path_obj) {
    size_t pl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    return ViCell_vfs_mkdir((const uint8_t *)path, pl)
           ? mp_const_true : mp_const_false;
}
static MP_DEFINE_CONST_FUN_OBJ_1(vfs_mkdir_obj, vfs_mkdir);

/* ── vfs.stat(path) -> (size:int, is_dir:bool) | None ───────────────────── */
static mp_obj_t vfs_stat(mp_obj_t path_obj) {
    size_t pl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    uint64_t sz = 0;
    int is_dir = 0;
    if (!ViCell_vfs_stat((const uint8_t *)path, pl, &sz, &is_dir)) {
        return mp_const_none;
    }
    mp_obj_t items[2] = {
        mp_obj_new_int_from_uint((mp_uint_t)sz),
        mp_obj_new_bool(is_dir),
    };
    return mp_obj_new_tuple(2, items);
}
static MP_DEFINE_CONST_FUN_OBJ_1(vfs_stat_obj, vfs_stat);

/* ── vfs.listdir(path) -> list[str] | None ──────────────────────────────── */
/*
 * Parses the "d:name\nf:name\n" buffer from VfsResponse::Data into a Python
 * list.  Zero-length segments (from trailing \n) are skipped.
 */
static mp_obj_t vfs_listdir(mp_obj_t path_obj) {
    size_t pl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    size_t n = ViCell_vfs_listdir((const uint8_t *)path, pl, s_listdir_buf, sizeof(s_listdir_buf));
    if (n == 0) return mp_const_none;

    mp_obj_t list = mp_obj_new_list(0, NULL);
    const char *start = (const char *)s_listdir_buf;
    const char *end   = start + n;
    const char *p     = start;
    while (p <= end) {
        if (p == end || *p == '\n') {
            size_t seg_len = (size_t)(p - start);
            if (seg_len > 0) {
                mp_obj_list_append(list, mp_obj_new_str(start, seg_len));
            }
            start = p + 1;
        }
        p++;
    }
    return list;
}
static MP_DEFINE_CONST_FUN_OBJ_1(vfs_listdir_obj, vfs_listdir);

/* ── vfs.remove(path) -> bool ────────────────────────────────────────────── */
static mp_obj_t vfs_remove(mp_obj_t path_obj) {
    size_t pl;
    const char *path = mp_obj_str_get_data(path_obj, &pl);
    return ViCell_vfs_remove((const uint8_t *)path, pl)
           ? mp_const_true : mp_const_false;
}
static MP_DEFINE_CONST_FUN_OBJ_1(vfs_remove_obj, vfs_remove);

/* ── Module table ────────────────────────────────────────────────────────── */
static const mp_rom_map_elem_t vfs_module_globals_table[] = {
    { MP_ROM_QSTR(MP_QSTR___name__), MP_ROM_QSTR(MP_QSTR_vfs)       },
    { MP_ROM_QSTR(MP_QSTR_read),     MP_ROM_PTR(&vfs_read_obj)      },
    { MP_ROM_QSTR(MP_QSTR_write),    MP_ROM_PTR(&vfs_write_obj)     },
    { MP_ROM_QSTR(MP_QSTR_append),   MP_ROM_PTR(&vfs_append_obj)    },
    { MP_ROM_QSTR(MP_QSTR_mkdir),    MP_ROM_PTR(&vfs_mkdir_obj)     },
    { MP_ROM_QSTR(MP_QSTR_stat),     MP_ROM_PTR(&vfs_stat_obj)      },
    { MP_ROM_QSTR(MP_QSTR_listdir),  MP_ROM_PTR(&vfs_listdir_obj)   },
    { MP_ROM_QSTR(MP_QSTR_remove),   MP_ROM_PTR(&vfs_remove_obj)    },
};
static MP_DEFINE_CONST_DICT(vfs_module_globals, vfs_module_globals_table);

const mp_obj_module_t mp_module_vfs_ViCell = {
    .base    = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&vfs_module_globals,
};

/* Register as the built-in module `vfs`. */
MP_REGISTER_MODULE(MP_QSTR_vfs, mp_module_vfs_ViCell);
