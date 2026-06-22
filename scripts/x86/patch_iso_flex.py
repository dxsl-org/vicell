#!/usr/bin/env python3
"""Patch an ISO9660 file by replacing a file with a same-or-smaller-sector-count ELF.

Unlike patch_iso.py (strict size match), this allows the new file to differ in byte size
as long as it fits within the SAME allocated sector count (ceil(size/2048)).
The directory entry's data-length field is updated to match the new size.

Usage: python patch_iso_flex.py <iso_file> <iso_path> <replacement_file>
Example: python patch_iso_flex.py Cellos-x86.iso /boot/kernel.elf new_kernel.elf
"""
import sys
import struct
import os
import math


def sectors(n):
    return math.ceil(n / 2048)


def read_le32(data, offset):
    return struct.unpack_from('<I', data, offset)[0]


def find_file_in_iso(iso_path: str, target_path: str):
    """Returns (byte_offset, size, dir_entry_offset_in_iso) of the file in the ISO."""
    with open(iso_path, 'rb') as f:
        f.seek(16 * 2048)
        pvd = f.read(2048)

    assert pvd[0] == 1 and pvd[1:6] == b'CD001'

    root_record_offset = 156
    root_extent_lba = read_le32(pvd, root_record_offset + 2)
    root_size        = read_le32(pvd, root_record_offset + 10)

    parts = [p for p in target_path.split('/') if p]
    current_lba  = root_extent_lba
    current_size = root_size

    for i, part in enumerate(parts):
        is_last = (i == len(parts) - 1)
        with open(iso_path, 'rb') as f:
            f.seek(current_lba * 2048)
            dir_data = f.read(current_size)
        # track the file offset of current_lba sector in the ISO
        dir_sector_offset = current_lba * 2048

        offset = 0
        found = False
        while offset < len(dir_data):
            rec_len = dir_data[offset]
            if rec_len == 0:
                offset = (offset // 2048 + 1) * 2048
                if offset >= len(dir_data):
                    break
                continue

            ext_lba  = read_le32(dir_data, offset + 2)
            ext_size = read_le32(dir_data, offset + 10)
            name_len = dir_data[offset + 32]
            name_raw = dir_data[offset + 33 : offset + 33 + name_len]

            name = name_raw.decode('ascii', errors='ignore')
            if ';' in name:
                name = name[:name.index(';')]
            if name.endswith('.'):
                name = name[:-1]

            if name.upper() == part.upper():
                if is_last:
                    dir_entry_iso_offset = dir_sector_offset + offset
                    return ext_lba * 2048, ext_size, dir_entry_iso_offset
                else:
                    current_lba  = ext_lba
                    current_size = ext_size
                    found = True
                    break

            offset += rec_len

        if not found and not is_last:
            raise FileNotFoundError(f"Directory component '{part}' not found in ISO")
        if not found and is_last:
            raise FileNotFoundError(f"File '{part}' not found")

    raise FileNotFoundError(f"Path not found: {target_path}")


def patch_iso(iso_file: str, iso_path: str, replacement_file: str):
    new_data = open(replacement_file, 'rb').read()
    byte_offset, stored_size, dir_entry_offset = find_file_in_iso(iso_file, iso_path)

    print(f"Found '{iso_path}' at byte offset {byte_offset:#x} ({byte_offset // 2048} sectors)")
    print(f"  Stored size:      {stored_size} bytes ({sectors(stored_size)} sectors)")
    print(f"  Replacement size: {len(new_data)} bytes ({sectors(len(new_data))} sectors)")
    print(f"  Dir entry offset: {dir_entry_offset:#x}")

    if sectors(len(new_data)) > sectors(stored_size):
        raise ValueError(
            f"Replacement uses {sectors(len(new_data))} sectors but only {sectors(stored_size)} are allocated. "
            "Rebuild ISO instead."
        )

    with open(iso_file, 'r+b') as f:
        # Write new ELF data
        f.seek(byte_offset)
        f.write(new_data)
        # Zero-pad remainder of last sector if we shrank
        pad = sectors(len(new_data)) * 2048 - len(new_data)
        if pad:
            f.write(b'\x00' * pad)

        # Update the directory record data-length fields (LE32 at +10, BE32 at +14)
        f.seek(dir_entry_offset + 10)
        f.write(struct.pack('<I', len(new_data)))   # LE32
        f.write(struct.pack('>I', len(new_data)))   # BE32 (ISO9660 has both)

    print(f"Patched {len(new_data)} bytes + updated dir entry size field.")


if __name__ == '__main__':
    if len(sys.argv) != 4:
        print(f"Usage: {sys.argv[0]} <iso_file> <iso_path> <replacement_file>")
        sys.exit(1)
    patch_iso(sys.argv[1], sys.argv[2], sys.argv[3])
