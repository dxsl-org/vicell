#!/usr/bin/env python3
"""
Generate MicroPython genhdr/ files via regex scanning (no C compiler needed).

Produces the same 4 files that MicroPython's Makefile-based build generates:
  genhdr/qstrdefs.generated.h  — QDEF0/QDEF1 table (hash + len + string)
  genhdr/mpversion.h           — version defines
  genhdr/moduledefs.h          — MP_REGISTER_MODULE stubs
  genhdr/root_pointers.h       — MP_REGISTER_ROOT_POINTER stubs

The hash function and static_qstr_list are taken verbatim from
vendor/py/makeqstrdata.py so the output is byte-for-byte compatible.

Usage: python gen_genhdr.py <vendor_dir> <output_genhdr_dir>
"""

import re
import os
import sys
from html.entities import codepoint2name

# ── Character escape map (mirrors makeqstrdata.py) ────────────────────────
codepoint2name[ord("-")]  = "hyphen"
codepoint2name[ord(" ")]  = "space"
codepoint2name[ord("'")]  = "squot"
codepoint2name[ord(",")]  = "comma"
codepoint2name[ord(".")]  = "dot"
codepoint2name[ord(":")]  = "colon"
codepoint2name[ord(";")]  = "semicolon"
codepoint2name[ord("/")]  = "slash"
codepoint2name[ord("%")]  = "percent"
codepoint2name[ord("#")]  = "hash"
codepoint2name[ord("(")]  = "paren_open"
codepoint2name[ord(")")]  = "paren_close"
codepoint2name[ord("[")]  = "bracket_open"
codepoint2name[ord("]")]  = "bracket_close"
codepoint2name[ord("{")]  = "brace_open"
codepoint2name[ord("}")]  = "brace_close"
codepoint2name[ord("*")]  = "star"
codepoint2name[ord("!")]  = "bang"
codepoint2name[ord("\\")] = "backslash"
codepoint2name[ord("+")]  = "plus"
codepoint2name[ord("$")]  = "dollar"
codepoint2name[ord("=")]  = "equals"
codepoint2name[ord("?")]  = "question"
codepoint2name[ord("@")]  = "at_sign"
codepoint2name[ord("^")]  = "caret"
codepoint2name[ord("|")]  = "pipe"
codepoint2name[ord("~")]  = "tilde"
codepoint2name[ord("\n")] = "newline"

# ── Static qstr list (verbatim from makeqstrdata.py, order matters for .mpy) ─
STATIC_QSTRS = [
    "",
    "__dir__",
    "\n",
    " ",
    "*",
    "/",
    "<module>",
    "_",
    "__call__",
    "__class__",
    "__delitem__",
    "__enter__",
    "__exit__",
    "__getattr__",
    "__getitem__",
    "__hash__",
    "__init__",
    "__int__",
    "__iter__",
    "__len__",
    "__main__",
    "__module__",
    "__name__",
    "__new__",
    "__next__",
    "__qualname__",
    "__repr__",
    "__setitem__",
    "__str__",
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException",
    "EOFError", "Ellipsis", "Exception", "GeneratorExit", "ImportError",
    "IndentationError", "IndexError", "KeyError", "KeyboardInterrupt",
    "LookupError", "MemoryError", "NameError", "NoneType",
    "NotImplementedError", "OSError", "OverflowError", "RuntimeError",
    "StopIteration", "SyntaxError", "SystemExit", "TypeError",
    "ValueError", "ZeroDivisionError",
    "abs", "all", "any", "append", "args", "bool", "builtins",
    "bytearray", "bytecode", "bytes", "callable", "chr", "classmethod",
    "clear", "close", "const", "copy", "count", "dict", "dir", "divmod",
    "end", "endswith", "eval", "exec", "extend", "find", "format",
    "from_bytes", "get", "getattr", "globals", "hasattr", "hash", "id",
    "index", "insert", "int", "isalpha", "isdigit", "isinstance",
    "islower", "isspace", "issubclass", "isupper", "items", "iter",
    "join", "key", "keys", "len", "list", "little", "locals", "lower",
    "lstrip", "main", "map", "micropython", "next", "object", "open",
    "ord", "pop", "popitem", "pow", "print", "range", "read", "readinto",
    "readline", "remove", "replace", "repr", "reverse", "rfind", "rindex",
    "round", "rsplit", "rstrip", "self", "send", "sep", "set", "setattr",
    "setdefault", "sort", "sorted", "split", "start", "startswith",
    "staticmethod", "step", "stop", "str", "strip", "sum", "super",
    "throw", "to_bytes", "tuple", "type", "update", "upper", "utf-8",
    "value", "values", "write", "zip",
]

# ── Unsorted pool (small index, from makeqstrdata.py) ────────────────────
UNSORTED_QSTRS = {
    "__bool__", "__pos__", "__neg__", "__invert__", "__abs__",
    "__float__", "__complex__", "__sizeof__",
    "__lt__", "__gt__", "__eq__", "__le__", "__ge__", "__ne__",
    "__contains__", "__iadd__", "__isub__", "__imul__", "__imatmul__",
    "__ifloordiv__", "__itruediv__", "__imod__", "__ipow__",
    "__ior__", "__ixor__", "__iand__", "__ilshift__", "__irshift__",
    "__add__", "__sub__", "__mul__", "__matmul__", "__floordiv__",
    "__truediv__", "__mod__", "__divmod__", "__pow__",
    "__or__", "__xor__", "__and__", "__lshift__", "__rshift__",
    "__radd__", "__rsub__", "__rmul__", "__rmatmul__", "__rfloordiv__",
    "__rtruediv__", "__rmod__", "__rpow__", "__ror__", "__rxor__",
    "__rand__", "__rlshift__", "__rrshift__",
    "__get__", "__set__", "__delete__",
    "<lambda>", "<listcomp>", "<dictcomp>", "<setcomp>", "<genexpr>",
}

QSTR_PATTERN    = re.compile(r'\bMP_QSTR_([A-Za-z0-9_]+)\b')
MODULE_PATTERN  = re.compile(
    r'(MP_REGISTER_MODULE|MP_REGISTER_EXTENSIBLE_MODULE)\s*\(\s*MP_QSTR_(\w+)\s*,\s*(\w+)\s*\)\s*;',
    re.DOTALL,
)
ROOT_PTR_PATTERN = re.compile(r'MP_REGISTER_ROOT_POINTER\s*\((.+?)\)\s*;', re.DOTALL)
SCAN_DIRS  = ['py', 'shared/runtime', 'extmod']
SCAN_EXTS  = {'.c', '.h'}

# Default qstr config (matches py/mpconfig.h defaults)
BYTES_IN_HASH = 2
BYTES_IN_LEN  = 1


def qstr_escape(s: str) -> str:
    def esc(m):
        c = ord(m.group(0))
        try:
            return "_" + codepoint2name[c] + "_"
        except KeyError:
            return "_0x%02x_" % c
    return re.sub(r"[^A-Za-z0-9_]", esc, s)


def compute_hash(s: str, bytes_hash: int) -> int:
    """Mirror of qstr.c / makeqstrdata.py hash function."""
    h = 5381
    for b in s.encode('utf-8'):
        h = (h * 33) ^ b
    mask = (1 << (8 * bytes_hash)) - 1
    return (h & mask) or 1


def escape_str(s: str) -> str:
    b = s.encode('utf-8')
    if all(32 <= c <= 126 and chr(c) not in '\\"' for c in b):
        return s
    return ''.join('\\x%02x' % c for c in b)


def make_qdef(ident: str, s: str, pool: int) -> str:
    b     = s.encode('utf-8')
    qhash = compute_hash(s, BYTES_IN_HASH)
    qlen  = len(b)
    qdata = escape_str(s)
    return 'QDEF%d(MP_QSTR_%s, %d, %d, "%s")' % (pool, ident, qhash, qlen, qdata)


def scan_qstrs_from_source(vendor_dir: str) -> set:
    """Regex-scan C/H files for MP_QSTR_xxx usages."""
    found = set()
    for scan_dir in SCAN_DIRS:
        d = os.path.join(vendor_dir, scan_dir)
        if not os.path.isdir(d):
            continue
        for fname in os.listdir(d):
            if os.path.splitext(fname)[1] not in SCAN_EXTS:
                continue
            try:
                text = open(os.path.join(d, fname), encoding='utf-8', errors='replace').read()
                for m in QSTR_PATTERN.finditer(text):
                    found.add(m.group(1))
            except Exception:
                pass
    return found


def write_qstrdefs(out_path: str, vendor_dir: str):
    # Track every ident emitted so far to prevent redeclaration.
    emitted: set[str] = {'MP_QSTRnull'}

    lines = ['// Automatically generated by gen_genhdr.py — do not edit.\n', '\n']
    lines.append('QDEF0(MP_QSTRnull, 0, 0, "")\n')

    # Pool 0a: static qstrs (ordered — order matters for .mpy compatibility)
    static_idents: set[str] = set()
    for s in STATIC_QSTRS:
        ident = qstr_escape(s)
        if ident in emitted:
            continue
        lines.append(make_qdef(ident, s, 0) + '\n')
        emitted.add(ident)
        static_idents.add(ident)

    # Pool 0b: unsorted qstrs (small-index requirement)
    for s in sorted(UNSORTED_QSTRS):
        ident = qstr_escape(s)
        if ident in emitted:
            continue
        lines.append(make_qdef(ident, s, 0) + '\n')
        emitted.add(ident)

    # Pool 1: extra qstrs found in source via MP_QSTR_xxx scan.
    # The raw identifier after MP_QSTR_ is already in escaped form for
    # pure-ASCII names.  Skip anything already in QDEF0 pools.
    raw_scanned = scan_qstrs_from_source(vendor_dir)
    extra_count = 0
    for raw_ident in sorted(raw_scanned):
        if raw_ident in emitted:
            continue
        lines.append(make_qdef(raw_ident, raw_ident, 1) + '\n')
        emitted.add(raw_ident)
        extra_count += 1

    with open(out_path, 'w') as f:
        f.writelines(lines)
    print(f'  qstrdefs.generated.h: {len(STATIC_QSTRS)} static + {len(UNSORTED_QSTRS)} unsorted + {extra_count} dynamic')


def write_mpversion(out_path: str, vendor_dir: str):
    major, minor, patch = 1, 24, 1
    ver_file = os.path.join(vendor_dir, 'py', 'mpconfig.h')
    if os.path.exists(ver_file):
        text = open(ver_file).read()
        for name, default, store in [
            ('MICROPY_VERSION_MAJOR', major, None),
            ('MICROPY_VERSION_MINOR', minor, None),
            ('MICROPY_VERSION_PATCH', patch, None),
        ]:
            m = re.search(rf'#define\s+{name}\s+\((\d+)\)', text)
            if m:
                val = int(m.group(1))
                if 'MAJOR' in name: major = val
                elif 'MINOR' in name: minor = val
                else: patch = val

    content = f"""\
// Automatically generated by gen_genhdr.py
#define MICROPY_VERSION_MAJOR {major}
#define MICROPY_VERSION_MINOR {minor}
#define MICROPY_VERSION_PATCH {patch}
#define MICROPY_VERSION_STRING "{major}.{minor}.{patch}"
#define MICROPY_GIT_TAG "v{major}.{minor}.{patch}"
#define MICROPY_GIT_HASH ""
#define MICROPY_BUILD_DATE ""
#define MICROPY_VERSION_PRERELEASE (0)
"""
    with open(out_path, 'w') as f:
        f.write(content)
    print(f'  mpversion.h: v{major}.{minor}.{patch}')


# Directories to scan for module/root-pointer registrations.
# extmod/ is excluded — it contains optional platform modules we don't compile.
_REGISTRATION_SCAN_DIRS = ['py', 'shared/runtime']


def _scan_all_c_files(vendor_dir: str):
    """Yield text content of .c files in registration scan dirs."""
    for scan_dir in _REGISTRATION_SCAN_DIRS:
        d = os.path.join(vendor_dir, scan_dir)
        if not os.path.isdir(d):
            continue
        for fname in os.listdir(d):
            if os.path.splitext(fname)[1] != '.c':
                continue
            try:
                yield open(os.path.join(d, fname), encoding='utf-8', errors='replace').read()
            except Exception:
                pass


def write_moduledefs(out_path: str, vendor_dir: str):
    """Mirror of makemoduledefs.py: generate extern + #define + MICROPY_REGISTERED_MODULES."""
    regular: list[tuple] = []    # (module_name, obj_module)
    extensible: list[tuple] = [] # (module_name, obj_module)

    seen = set()
    for text in _scan_all_c_files(vendor_dir):
        for macro, qstr_name, obj_module in MODULE_PATTERN.findall(text):
            key = (macro, qstr_name, obj_module)
            if key in seen:
                continue
            seen.add(key)
            if macro == 'MP_REGISTER_MODULE':
                regular.append((qstr_name, obj_module))
            else:
                extensible.append((qstr_name, obj_module))

    lines = ['// Automatically generated by gen_genhdr.py.\n\n']

    # Emit extern + #define for each module
    for qstr_name, obj in sorted(set(regular + extensible)):
        mod_def = 'MODULE_DEF_%s' % qstr_name.upper()
        lines.append('extern const struct _mp_obj_module_t %s;\n' % obj)
        lines.append('#undef %s\n' % mod_def)
        lines.append('#define %s { MP_ROM_QSTR(MP_QSTR_%s), MP_ROM_PTR(&%s) },\n\n'
                     % (mod_def, qstr_name, obj))

    # MICROPY_REGISTERED_MODULES
    lines.append('\n#define MICROPY_REGISTERED_MODULES \\\n')
    for qstr_name, _ in sorted(set(regular)):
        lines.append('    MODULE_DEF_%s \\\n' % qstr_name.upper())
    lines.append('// MICROPY_REGISTERED_MODULES\n')

    # MICROPY_REGISTERED_EXTENSIBLE_MODULES
    lines.append('\n#define MICROPY_REGISTERED_EXTENSIBLE_MODULES \\\n')
    for qstr_name, _ in sorted(set(extensible)):
        lines.append('    MODULE_DEF_%s \\\n' % qstr_name.upper())
    lines.append('// MICROPY_REGISTERED_EXTENSIBLE_MODULES\n')

    with open(out_path, 'w') as f:
        f.writelines(lines)
    print(f'  moduledefs.h: {len(regular)} regular + {len(extensible)} extensible modules')


def write_root_pointers(out_path: str, vendor_dir: str):
    """Mirror of make_root_pointers.py: bare C variable declarations inside mp_state_vm_t."""
    ptrs: set[str] = set()
    for text in _scan_all_c_files(vendor_dir):
        for decl in ROOT_PTR_PATTERN.findall(text):
            ptrs.add(decl.strip())

    lines = ['// Automatically generated by gen_genhdr.py.\n\n']
    for decl in sorted(ptrs):
        lines.append('%s;\n' % decl)

    with open(out_path, 'w') as f:
        f.writelines(lines)
    print(f'  root_pointers.h: {len(ptrs)} declarations')


def main():
    if len(sys.argv) < 3:
        print('Usage: gen_genhdr.py <vendor_dir> <genhdr_output_dir>')
        sys.exit(1)
    vendor_dir = sys.argv[1]
    out_dir    = sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)

    print('Generating genhdr/ files...')
    write_qstrdefs(os.path.join(out_dir, 'qstrdefs.generated.h'), vendor_dir)
    write_mpversion(os.path.join(out_dir, 'mpversion.h'),         vendor_dir)
    write_moduledefs(os.path.join(out_dir, 'moduledefs.h'),       vendor_dir)
    write_root_pointers(os.path.join(out_dir, 'root_pointers.h'), vendor_dir)
    print('Done.')


if __name__ == '__main__':
    main()
