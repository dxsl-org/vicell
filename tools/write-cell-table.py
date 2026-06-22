#!/usr/bin/env python3
"""Append the Cellos cell bootstrap table to an existing disk image.

The bootstrap table is appended at CELL_TABLE_BASE_LBA (526336 sectors = ~257 MB offset),
AFTER the primary FAT32 partition.  The FAT32 filesystem is not affected.

Usage:
    python write-cell-table.py <disk_image> <path1>=<elf1> [<path2>=<elf2> ...]

Example:
    python write-cell-table.py disk_v3.img /bin/vfs=target/.../vfs /bin/shell=target/.../shell
"""
import struct
import sys
import os

SECTOR_SIZE       = 512
CELL_TABLE_BASE_LBA = 526_336         # must match disk_layout.rs
CELL_TABLE_MAGIC  = 0x5649_4F53_5F43_454C  # "Cellos_CEL"
CELL_PATH_LEN     = 64
MAX_CELL_ENTRIES  = 32

# ── Layout helpers ────────────────────────────────────────────────────────────

def pack_header(count: int) -> bytes:
    """CellTableHeader: magic(8) + count(4) + pad(500) = 512 bytes."""
    return struct.pack("<QI", CELL_TABLE_MAGIC, count) + b"\x00" * 500

def pack_entry(path: str, data_lba: int, data_size: int) -> bytes:
    """CellEntry: path(64) + data_lba(8) + data_size(8) + pad(432) = 512 bytes."""
    path_bytes = path.encode("utf-8")
    # Must fit in CELL_PATH_LEN - 1 bytes to leave room for a null terminator.
    if len(path_bytes) >= CELL_PATH_LEN:
        raise ValueError(
            f"Path '{path}' encodes to {len(path_bytes)} bytes "
            f"but max is {CELL_PATH_LEN - 1} (need 1 byte for NUL terminator)"
        )
    path_bytes = path_bytes.ljust(CELL_PATH_LEN, b"\x00")
    return path_bytes + struct.pack("<QQ", data_lba, data_size) + b"\x00" * 432

def sectors_for(size: int) -> int:
    return (size + SECTOR_SIZE - 1) // SECTOR_SIZE

# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <disk.img> <path1>=<elf1> ...", file=sys.stderr)
        sys.exit(1)

    disk_img = sys.argv[1]
    pairs = sys.argv[2:]

    cells: list[tuple[str, str]] = []
    for pair in pairs:
        if "=" not in pair:
            print(f"ERROR: expected path=elf_file, got {pair!r}", file=sys.stderr)
            sys.exit(1)
        cell_path, elf_file = pair.split("=", 1)
        if not os.path.isfile(elf_file):
            print(f"WARN: ELF not found: {elf_file} — skipping {cell_path}", file=sys.stderr)
            continue
        cells.append((cell_path, elf_file))

    if not cells:
        print("No cell ELFs found — bootstrap table will be empty.", file=sys.stderr)
        return

    if len(cells) > MAX_CELL_ENTRIES:
        print(f"ERROR: too many cells ({len(cells)} > {MAX_CELL_ENTRIES})", file=sys.stderr)
        sys.exit(1)

    # Ensure disk image is large enough (pad if needed).
    min_size = (CELL_TABLE_BASE_LBA + 1 + MAX_CELL_ENTRIES) * SECTOR_SIZE
    with open(disk_img, "r+b") as f:
        f.seek(0, 2)
        current_size = f.tell()
        if current_size < min_size:
            f.write(b"\x00" * (min_size - current_size))

    # Load ELF data.
    elf_data_list: list[tuple[str, int, bytes]] = []  # (path, data_lba, data)
    data_start_lba = CELL_TABLE_BASE_LBA + 1 + len(cells)
    current_lba = data_start_lba

    for cell_path, elf_file in cells:
        with open(elf_file, "rb") as ef:
            data = ef.read()
        elf_data_list.append((cell_path, current_lba, data))
        current_lba += sectors_for(len(data))

    # Write table to disk image.
    total_sectors_needed = current_lba * SECTOR_SIZE
    with open(disk_img, "r+b") as f:
        # Extend image if necessary.
        f.seek(0, 2)
        if f.tell() < total_sectors_needed:
            f.write(b"\x00" * (total_sectors_needed - f.tell()))

        # Header.
        f.seek(CELL_TABLE_BASE_LBA * SECTOR_SIZE)
        f.write(pack_header(len(cells)))

        # Entry table.
        for cell_path, data_lba, data in elf_data_list:
            f.write(pack_entry(cell_path, data_lba, len(data)))

        # ELF data (sector-padded).
        for _, _, data in elf_data_list:
            f.write(data)
            remainder = len(data) % SECTOR_SIZE
            if remainder:
                f.write(b"\x00" * (SECTOR_SIZE - remainder))

    print(f"[cell-table] wrote {len(cells)} entries to {disk_img}:")
    for cell_path, data_lba, data in elf_data_list:
        print(f"  {cell_path:<32} @ LBA {data_lba:>8}  ({len(data):>8} bytes)")

if __name__ == "__main__":
    main()
