"""mkfat16.py — Write an empty FAT16 filesystem to LBA 0 of an existing image.

Usage:
    python mkfat16.py <image_path> <total_sectors>

Unlike mkfat32.py (which creates a standalone image with files), this writes
IN-PLACE into an already-allocated disk image WITHOUT extending it. The caller
is responsible for ensuring the image is large enough and that `total_sectors`
does not overlap the cell bootstrap table at LBA 82000.

FAT16 geometry at 8 sectors/cluster (4096-byte clusters) on 81920 sectors:
- ~10 200 data clusters → inside the FAT16 window 4085..65524
- Compatible with the fatfs crate's FAT16 auto-detection

This script is idempotent: re-running it overwrites the BPB and FATs but leaves
data-region sectors untouched (they are always overwritten by runtime writes).
"""

import struct
import sys

SECTOR_SIZE          = 512
SEC_PER_CLUS         = 8            # 4096-byte clusters
RESERVED_SECTORS     = 1
NUM_FATS             = 2
ROOT_ENTRIES         = 512          # 32 B each → 32 root-dir sectors
CELL_TABLE_BASE_LBA  = 82_000       # must match disk_layout.rs

def fat16_geometry(total_sectors: int):
    """Return (fat_sectors, root_dir_sectors, data_start_lba, cluster_count)."""
    root_dir_sectors = (ROOT_ENTRIES * 32 + SECTOR_SIZE - 1) // SECTOR_SIZE  # 32

    # Iterate to converge FAT size (FAT16 entries are 2 bytes each).
    fat_sectors = 1
    for _ in range(16):
        data_region = total_sectors - RESERVED_SECTORS - NUM_FATS * fat_sectors - root_dir_sectors
        clusters = data_region // SEC_PER_CLUS
        new_fat = ((clusters + 2) * 2 + SECTOR_SIZE - 1) // SECTOR_SIZE
        if new_fat == fat_sectors:
            break
        fat_sectors = new_fat

    data_start = RESERVED_SECTORS + NUM_FATS * fat_sectors + root_dir_sectors
    clusters   = (total_sectors - data_start) // SEC_PER_CLUS

    if not (4085 <= clusters <= 65524):
        raise SystemExit(f"[mkfat16] ERROR: cluster count {clusters} outside FAT16 window "
                         f"4085..65524; adjust total_sectors or SEC_PER_CLUS")
    return fat_sectors, root_dir_sectors, data_start, clusters


def main():
    if len(sys.argv) != 3:
        raise SystemExit("Usage: python mkfat16.py <image_path> <total_sectors>")

    img_path      = sys.argv[1]
    total_sectors = int(sys.argv[2])

    # Guard: ensure we stay clear of the cell bootstrap table.
    if total_sectors > CELL_TABLE_BASE_LBA - 1:
        raise SystemExit(f"[mkfat16] ERROR: total_sectors {total_sectors} would reach "
                         f"LBA {total_sectors} >= CELL_TABLE_BASE_LBA {CELL_TABLE_BASE_LBA}")

    fat_sectors, root_dir_sectors, data_start, clusters = fat16_geometry(total_sectors)

    # ── Boot sector (BPB) ──────────────────────────────────────────────────────
    boot = bytearray(SECTOR_SIZE)
    # Jump instruction + OEM name
    boot[0:3]   = b'\xEB\x3C\x90'
    boot[3:11]  = b'MSWIN4.1'
    # BPB
    struct.pack_into('<H', boot, 11, SECTOR_SIZE)     # BytesPerSector
    boot[13]    = SEC_PER_CLUS                         # SectorsPerCluster
    struct.pack_into('<H', boot, 14, RESERVED_SECTORS) # ReservedSectors
    boot[16]    = NUM_FATS                             # NumFATs
    struct.pack_into('<H', boot, 17, ROOT_ENTRIES)    # RootEntCnt
    # TotSec16: 0 if >= 65536, else total_sectors
    struct.pack_into('<H', boot, 19, total_sectors if total_sectors < 0x10000 else 0)
    boot[21]    = 0xF8                                 # Media: fixed disk
    struct.pack_into('<H', boot, 22, fat_sectors)     # FATSz16
    struct.pack_into('<H', boot, 24, 63)              # SecPerTrk (irrelevant for image)
    struct.pack_into('<H', boot, 26, 255)             # NumHeads  (irrelevant for image)
    struct.pack_into('<I', boot, 28, 0)               # HiddSec
    struct.pack_into('<I', boot, 32, total_sectors if total_sectors >= 0x10000 else 0)  # TotSec32
    # Extended BPB (FAT16)
    boot[36]    = 0x80                                 # DrvNum
    boot[37]    = 0x00                                 # Reserved1
    boot[38]    = 0x29                                 # BootSig
    struct.pack_into('<I', boot, 39, 0x56494F44)       # VolID "VIOD"
    boot[43:54] = b'ViOS DATA  '                       # VolLab (11 bytes)
    boot[54:62] = b'FAT16   '                          # FilSysType (8 bytes)
    boot[510:512] = b'\x55\xAA'

    # ── FAT table (both copies) ────────────────────────────────────────────────
    # Entry 0: media byte + 0xFF; entry 1: EOC. All others 0x0000 (free).
    fat = bytearray(fat_sectors * SECTOR_SIZE)
    struct.pack_into('<H', fat, 0, 0xFFF8)  # FAT[0] = media descriptor
    struct.pack_into('<H', fat, 2, 0xFFFF)  # FAT[1] = EOC

    # ── Root directory region (zeroed → empty) ─────────────────────────────────
    root_dir = bytearray(root_dir_sectors * SECTOR_SIZE)

    # ── Write IN-PLACE at LBA 0 (r+b — never extends the file) ───────────────
    with open(img_path, 'r+b') as f:
        # Boot sector at LBA 0
        f.seek(0)
        f.write(boot)
        # FAT1 at LBA RESERVED_SECTORS
        f.seek(RESERVED_SECTORS * SECTOR_SIZE)
        f.write(fat)
        # FAT2 at LBA RESERVED_SECTORS + fat_sectors
        f.seek((RESERVED_SECTORS + fat_sectors) * SECTOR_SIZE)
        f.write(fat)
        # Root dir at LBA RESERVED_SECTORS + NUM_FATS*fat_sectors
        root_dir_start = RESERVED_SECTORS + NUM_FATS * fat_sectors
        f.seek(root_dir_start * SECTOR_SIZE)
        f.write(root_dir)

    print(f"[mkfat16] {img_path}: {total_sectors} sectors, "
          f"{clusters} data clusters (FAT16), FATsz={fat_sectors}, "
          f"data start=LBA {data_start}")


if __name__ == '__main__':
    main()
