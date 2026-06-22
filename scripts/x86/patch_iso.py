#!/usr/bin/env python3
"""Patch an ISO9660 file in-place by replacing a file with same-size content.

Usage: python patch_iso.py <iso_file> <iso_path> <replacement_file>
Example: python patch_iso.py Cellos-x86.iso /boot/kernel.elf new_kernel.elf
"""
import sys
import struct
import os


def read_le32(data, offset):
    return struct.unpack_from('<I', data, offset)[0]


def find_file_in_iso(iso_path: str, target_path: str):
    """Returns (byte_offset, size) of the file in the ISO.
    target_path should be like '/boot/kernel.elf' (leading slash).
    """
    with open(iso_path, 'rb') as f:
        # Sector 16 = Primary Volume Descriptor
        f.seek(16 * 2048)
        pvd = f.read(2048)

    assert pvd[0] == 1, "Not a Primary Volume Descriptor"
    assert pvd[1:6] == b'CD001', "Not a valid ISO9660 PVD"

    # Root directory record is at offset 156 in PVD
    root_record_offset = 156
    root_extent_lba = read_le32(pvd, root_record_offset + 2)
    root_size        = read_le32(pvd, root_record_offset + 10)

    # Traverse path components
    parts = [p for p in target_path.split('/') if p]
    current_lba  = root_extent_lba
    current_size = root_size

    for i, part in enumerate(parts):
        is_last = (i == len(parts) - 1)
        # Read the directory at current_lba
        with open(iso_path, 'rb') as f:
            f.seek(current_lba * 2048)
            dir_data = f.read(current_size)

        offset = 0
        found = False
        while offset < len(dir_data):
            rec_len = dir_data[offset]
            if rec_len == 0:
                # Advance to next sector
                offset = (offset // 2048 + 1) * 2048
                if offset >= len(dir_data):
                    break
                continue

            ext_lba  = read_le32(dir_data, offset + 2)
            ext_size = read_le32(dir_data, offset + 10)
            name_len = dir_data[offset + 32]
            name_raw = dir_data[offset + 33 : offset + 33 + name_len]

            # Strip version suffix (e.g. ";1")
            name = name_raw.decode('ascii', errors='ignore')
            if ';' in name:
                name = name[:name.index(';')]
            # Trailing dot means no extension
            if name.endswith('.'):
                name = name[:-1]

            if name.upper() == part.upper():
                if is_last:
                    return ext_lba * 2048, ext_size
                else:
                    current_lba  = ext_lba
                    current_size = ext_size
                    found = True
                    break

            offset += rec_len

        if not found and not is_last:
            raise FileNotFoundError(f"Directory component '{part}' not found in ISO")
        if not found and is_last:
            raise FileNotFoundError(f"File '{part}' not found in ISO directory")

    raise FileNotFoundError(f"Path not found: {target_path}")


def patch_iso(iso_file: str, iso_path: str, replacement_file: str):
    new_data = open(replacement_file, 'rb').read()
    byte_offset, stored_size = find_file_in_iso(iso_file, iso_path)

    print(f"Found '{iso_path}' at byte offset {byte_offset:#x} ({byte_offset // 2048} sectors), size {stored_size}")

    if len(new_data) > stored_size:
        raise ValueError(
            f"New file ({len(new_data)} bytes) is LARGER than slot in ISO ({stored_size} bytes). "
            "Cannot patch in-place — rebuild the ISO."
        )
    if len(new_data) < stored_size:
        # Pad with zeros so the write fills the original ISO sector allocation.
        # ELF loaders stop at the last section described in headers; trailing
        # zeros are ignored.  ISO9660 stores the unpadded size in its directory,
        # so Limine still parses the correct ELF size.
        pad = stored_size - len(new_data)
        new_data = new_data + bytes(pad)
        print(f"  (padded {pad} bytes with zeros to match ISO slot)")

    with open(iso_file, 'r+b') as f:
        f.seek(byte_offset)
        f.write(new_data)

    print(f"Patched {len(new_data)} bytes into '{iso_file}' at offset {byte_offset:#x}")


if __name__ == '__main__':
    if len(sys.argv) != 4:
        print(f"Usage: {sys.argv[0]} <iso_file> <iso_path> <replacement_file>")
        sys.exit(1)
    patch_iso(sys.argv[1], sys.argv[2], sys.argv[3])
