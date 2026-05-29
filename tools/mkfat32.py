"""mkfat32.py — Minimal FAT32 image creator with subdirectory support.

Usage:
    python mkfat32.py <output.img> [<src_path> <dst_path>] ...

Where dst_path can include directory components: e.g. /bin/init, /etc/hostname.
All intermediate directories are created automatically.

FAT32 geometry: 1 sector/cluster (512 bytes), auto-sized to fit all files.
"""

import struct
import os
import sys
from collections import defaultdict


# ── FAT32 constants ───────────────────────────────────────────────────────────

SECTOR_SIZE    = 512
CLUSTER_SIZE   = SECTOR_SIZE      # 1 sector per cluster
RESERVED       = 32               # reserved sectors (boot + FSInfo + backup)
FATS           = 2
ATTR_ARCHIVE   = 0x20
ATTR_DIRECTORY = 0x10
EOC            = 0x0FFFFFFF       # end-of-chain marker
DATE_DEFAULT   = 0x5700           # 2026-11-16 encoded (year-1980=46, mon=11, day=16)
TIME_DEFAULT   = 0x8C00           # 17:32:00


# ── Directory-entry helpers ───────────────────────────────────────────────────

def make_83_name(name: str) -> bytes:
    """Convert a simple name (no path) to 8.3 format, padded with spaces."""
    name = name.upper()
    if '.' in name:
        base, ext = name.rsplit('.', 1)
    else:
        base, ext = name, ''
    return (base[:8].ljust(8) + ext[:3].ljust(3)).encode('ascii')


def dir_entry(name83: bytes, cluster: int, size: int, attr: int = ATTR_ARCHIVE) -> bytes:
    """Build a 32-byte FAT32 directory entry."""
    high = (cluster >> 16) & 0xFFFF
    low  =  cluster        & 0xFFFF
    return (
        name83 +
        struct.pack('<B',  attr) +          # attributes
        struct.pack('<B',  0) +             # NTRes
        struct.pack('<B',  0) +             # CrtTimeTenth
        struct.pack('<H',  TIME_DEFAULT) +  # CrtTime
        struct.pack('<H',  DATE_DEFAULT) +  # CrtDate
        struct.pack('<H',  DATE_DEFAULT) +  # LstAccDate
        struct.pack('<H',  high) +          # FstClusHI
        struct.pack('<H',  TIME_DEFAULT) +  # WrtTime
        struct.pack('<H',  DATE_DEFAULT) +  # WrtDate
        struct.pack('<H',  low) +           # FstClusLO
        struct.pack('<I',  size)            # FileSize
    )


def dot_entries(self_cluster: int, parent_cluster: int) -> bytes:
    """. and .. entries for a subdirectory."""
    return (
        dir_entry(b'.          ', self_cluster,   0, ATTR_DIRECTORY) +
        dir_entry(b'..         ', parent_cluster, 0, ATTR_DIRECTORY)
    )


# ── Main ──────────────────────────────────────────────────────────────────────

def create_fat32_image(output_path: str, files: list[tuple[str, str]]):
    """
    Create a FAT32 disk image at *output_path* containing *files*.

    *files* is a list of (source_path, dest_path) pairs where dest_path may
    include directory components, e.g. '/bin/init' or '/etc/hostname'.
    """

    # ── 1. Load file data ────────────────────────────────────────────────────
    file_data = {}   # dest_path → bytes
    for src, dst in files:
        dst = dst.lstrip('/')   # normalise: remove leading slash
        if not os.path.exists(src):
            print(f"WARNING: {src} not found — skipped", file=sys.stderr)
            continue
        with open(src, 'rb') as fh:
            file_data[dst] = fh.read()

    # ── 2. Compute disk geometry ─────────────────────────────────────────────
    total_bytes = sum(len(d) for d in file_data.values())
    needed      = int(total_bytes * 1.2) + 2 * 1024 * 1024   # 2 MB FAT+dir overhead
    min_sectors = max(2048, (needed + SECTOR_SIZE - 1) // SECTOR_SIZE)
    sector_count = ((min_sectors + 8191) // 8192) * 8192      # align to 4 MB

    approx_clusters = sector_count
    fat_sectors = (approx_clusters * 4 + SECTOR_SIZE - 1) // SECTOR_SIZE
    fat_sectors = ((fat_sectors + 31) // 32) * 32             # align to 16 KB
    data_start  = RESERVED + FATS * fat_sectors               # first data sector

    # ── 3. Cluster allocator ─────────────────────────────────────────────────
    fat = bytearray(fat_sectors * SECTOR_SIZE)
    struct.pack_into('<I', fat, 0, 0x0FFFFFF8)   # entry 0
    struct.pack_into('<I', fat, 4, 0x0FFFFFFF)   # entry 1

    next_cluster = [2]   # mutable int via list

    def alloc_chain(num_clusters: int) -> int:
        """Allocate a chain of *num_clusters* clusters; return start cluster."""
        if num_clusters == 0:
            return 0
        start = next_cluster[0]
        for i in range(num_clusters):
            c = next_cluster[0]
            next_cluster[0] += 1
            nxt = next_cluster[0] if i < num_clusters - 1 else EOC
            struct.pack_into('<I', fat, c * 4, nxt)
        return start

    def cluster_offset(cluster: int) -> int:
        """Byte offset of *cluster* in the image."""
        return (data_start + (cluster - 2)) * SECTOR_SIZE

    # ── 4. Build directory tree ──────────────────────────────────────────────
    # dir_files[dir_path] → list of (filename, cluster, size, attr)
    dir_files = defaultdict(list)   # '' = root

    # Discover all directories needed
    dirs = {''}    # root always exists
    for dst in file_data:
        parts = dst.split('/')
        for i in range(len(parts) - 1):
            dirs.add('/'.join(parts[:i+1]))

    # Assign clusters to each directory (1 cluster = 16 entries max for first cluster)
    dir_cluster = {'': alloc_chain(1)}   # root at cluster 2
    for d in sorted(dirs):
        if d == '':
            continue
        dir_cluster[d] = alloc_chain(1)

    # Register subdirectories in their parent dirs
    for d in sorted(dirs):
        if d == '':
            continue
        parent = d.rsplit('/', 1)[0] if '/' in d else ''
        name   = d.rsplit('/', 1)[-1]
        dir_files[parent].append((name, dir_cluster[d], 0, ATTR_DIRECTORY))

    # Allocate clusters for files and register them in their parent dirs
    file_cluster = {}
    for dst, data in file_data.items():
        num_c = max(1, (len(data) + CLUSTER_SIZE - 1) // CLUSTER_SIZE)
        fc    = alloc_chain(num_c)
        file_cluster[dst] = fc
        parent = dst.rsplit('/', 1)[0] if '/' in dst else ''
        name   = dst.rsplit('/', 1)[-1]
        dir_files[parent].append((name, fc, len(data), ATTR_ARCHIVE))

    # ── 5. Write image ───────────────────────────────────────────────────────
    with open(output_path, 'wb+') as img:
        # a) Zero-fill
        img.seek(sector_count * SECTOR_SIZE - 1)
        img.write(b'\x00')
        img.seek(0)

        # b) Boot sector
        img.write(b'\xEB\x58\x90' + b'MSWIN4.1')
        img.write(struct.pack('<H', SECTOR_SIZE))
        img.write(struct.pack('<B', 1))             # sectors/cluster
        img.write(struct.pack('<H', RESERVED))
        img.write(struct.pack('<B', FATS))
        img.write(struct.pack('<H', 0))             # root entry count (FAT32=0)
        img.write(struct.pack('<H', 0))             # total sectors 16-bit (FAT32=0)
        img.write(b'\xF8')                          # media descriptor
        img.write(struct.pack('<H', 0))             # FAT size 16-bit (FAT32=0)
        img.write(struct.pack('<H', 63))            # sectors/track
        img.write(struct.pack('<H', 255))           # heads
        img.write(struct.pack('<I', 0))             # hidden sectors
        img.write(struct.pack('<I', sector_count))  # total sectors 32-bit
        # FAT32 extended BPB
        img.write(struct.pack('<I', fat_sectors))
        img.write(struct.pack('<H', 0))             # ext flags
        img.write(struct.pack('<H', 0))             # FS version
        img.write(struct.pack('<I', 2))             # root cluster
        img.write(struct.pack('<H', 1))             # FSInfo sector
        img.write(struct.pack('<H', 6))             # backup boot sector
        img.write(b'\x00' * 12)
        img.write(b'\x80\x00\x29')
        img.write(struct.pack('<I', 0x12345678))    # volume serial
        img.write(b'VIOS       ')                   # volume label
        img.write(b'FAT32   ')
        img.seek(510)
        img.write(b'\x55\xAA')

        # c) FSInfo (sector 1)
        img.seek(SECTOR_SIZE)
        img.write(b'RRaA' + b'\x00' * 480)
        img.seek(SECTOR_SIZE + 484)
        img.write(b'rrAa')
        img.write(struct.pack('<I', 0xFFFFFFFF))   # free count unknown
        img.write(struct.pack('<I', 0xFFFFFFFF))   # next free unknown
        img.write(b'\x00' * 12 + b'\x00\x00\x55\xAA')

        # d) Backup boot sector (sector 6)
        img.seek(0); bs = img.read(SECTOR_SIZE)
        img.seek(6 * SECTOR_SIZE); img.write(bs)

        # e) FAT tables
        img.seek(RESERVED * SECTOR_SIZE)
        img.write(fat)
        img.seek((RESERVED + fat_sectors) * SECTOR_SIZE)
        img.write(fat)

        # f) Directory entries
        for dir_path, entries in dir_files.items():
            dc    = dir_cluster[dir_path]
            off   = cluster_offset(dc)
            img.seek(off)

            # . and .. for non-root directories
            if dir_path != '':
                parent_path = dir_path.rsplit('/', 1)[0] if '/' in dir_path else ''
                img.write(dot_entries(dc, dir_cluster[parent_path]))

            for name, cluster, size, attr in entries:
                img.write(dir_entry(make_83_name(name), cluster, size, attr))

        # g) File data
        for dst, data in file_data.items():
            fc  = file_cluster[dst]
            off = cluster_offset(fc)
            img.seek(off)
            img.write(data)

    ndirs  = len(dirs) - 1  # exclude root
    nfiles = len(file_data)
    print(f"Created FAT32 image at {output_path}: "
          f"{nfiles} file(s), {ndirs} director(ies).")


if __name__ == '__main__':
    if len(sys.argv) < 2:
        print('Usage: mkfat32.py <output.img> [<src> <dst>] ...')
        sys.exit(1)
    out   = sys.argv[1]
    args  = sys.argv[2:]
    pairs = [(args[i], args[i+1]) for i in range(0, len(args) - 1, 2)]
    create_fat32_image(out, pairs)
