#!/usr/bin/env python3
"""Write the Cellos MBR partition table to LBA 0 of a disk image.

Layout (must match kernel/src/loader/disk_layout.rs and api::disk):

    P1  type 0x0C (FAT32 LBA)  @ LBA   2,048  size 524,288   /mnt/sd interop volume
    P2  type 0x7F (Cellos)     @ LBA 526,336  size  33,664   cell bootstrap table + ELF blobs
    P3  type 0x7D (Cellos)     @ LBA 560,000  size 240,000   kernel heap snapshot (Phase 29)
    P4  type 0x7E (Cellos)     @ LBA 800,000  size 131,072   littlefs /data (Milestone 2.5 P04)

Total disk: 931,072 sectors (~455 MB). The image is written sparsely by gen_disk.

Only the partition table bytes (446..510) and the 0x55AA signature are written —
the rest of LBA 0 is left untouched so this is safe to run before or after the
FAT32 formatter (which now writes its BPB at LBA 2048, not LBA 0).

Usage:
    python write-mbr.py <disk_image>
"""
import struct
import sys

SECTOR_SIZE = 512

# (type, start_lba, sectors) — order defines MBR slots 1-4.
PARTITIONS = [
    (0x0C,   2_048, 524_288),  # P1 FAT32 (LBA addressing)
    (0x7F, 526_336,  33_664),  # P2 Cellos cell table
    (0x7D, 560_000, 240_000),  # P3 Cellos snapshot
    (0x7E, 800_000, 131_072),  # P4 Cellos littlefs
]


def pack_entry(ptype: int, start: int, size: int) -> bytes:
    """One 16-byte MBR partition entry, LBA-only (CHS fields set to 0xFF)."""
    return struct.pack(
        "<B3sB3sII",
        0x00,                  # status: non-bootable (firmware boots the kernel directly)
        b"\xFF\xFF\xFF",      # CHS first (unused — LBA only)
        ptype,
        b"\xFF\xFF\xFF",      # CHS last (unused)
        start,
        size,
    )


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python write-mbr.py <disk_image>")
    img = sys.argv[1]

    table = b"".join(pack_entry(*p) for p in PARTITIONS)
    assert len(table) == 64

    with open(img, "r+b") as f:
        f.seek(446)
        f.write(table)
        f.seek(510)
        f.write(b"\x55\xAA")

    print("[write-mbr] MBR written:")
    for i, (ptype, start, size) in enumerate(PARTITIONS, 1):
        print(f"  P{i} type={ptype:#04x} start={start:>7} sectors={size:>7}")


if __name__ == "__main__":
    main()
