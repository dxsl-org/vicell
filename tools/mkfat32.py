"""mkfat32.py — Minimal FAT16 image creator with subdirectory support.

Usage:
    python mkfat32.py <output.img> [<src_path> <dst_path>] ...

Where dst_path can include directory components: e.g. /bin/init, /etc/hostname.
All intermediate directories are created automatically.

NOTE ON FAT TYPE: despite the historical filename, this emits **FAT16**, not
FAT32.  A spec-compliant FAT32 volume requires >= 65525 data clusters
(~34 MB at 512-byte clusters); our embedded image is only a few MB, so a FAT32
BPB on it is rejected by strict parsers (e.g. the `fatfs` crate →
CorruptedFileSystem).  FAT16 is valid for 4085..65524 clusters, which fits the
small embedded image while staying mountable.  The kernel's `fatfs` mount
auto-detects the FAT type from the cluster count, so no kernel change is needed.

FAT16 geometry: 1 sector/cluster (512 bytes), fixed-size root directory region,
auto-sized to fit all files.
"""

import struct
import os
import sys
from collections import defaultdict


# ── FAT16 constants ────────────────────────────────────────────────────────────

SECTOR_SIZE    = 512
CLUSTER_SIZE   = SECTOR_SIZE      # 1 sector per cluster
RESERVED       = 1                # FAT16 needs only the boot sector reserved
FATS           = 2
ROOT_ENTRIES   = 512              # fixed root directory capacity (512 * 32 = 32 sectors)
ATTR_ARCHIVE   = 0x20
ATTR_DIRECTORY = 0x10
EOC16          = 0xFFFF           # end-of-chain marker (FAT16)
DATE_DEFAULT   = 0x5700           # 2026-11-16 encoded (year-1980=46, mon=11, day=16)
TIME_DEFAULT   = 0x8C00           # 17:32:00

# FAT16 valid cluster-count window (outside → FAT12 or FAT32, which we must avoid).
FAT16_MIN_CLUSTERS = 4085
FAT16_MAX_CLUSTERS = 65524


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
    """Build a 32-byte directory entry (FAT16: FstClusHI is always 0)."""
    low = cluster & 0xFFFF
    return (
        name83 +
        struct.pack('<B',  attr) +          # attributes
        struct.pack('<B',  0) +             # NTRes
        struct.pack('<B',  0) +             # CrtTimeTenth
        struct.pack('<H',  TIME_DEFAULT) +  # CrtTime
        struct.pack('<H',  DATE_DEFAULT) +  # CrtDate
        struct.pack('<H',  DATE_DEFAULT) +  # LstAccDate
        struct.pack('<H',  0) +             # FstClusHI (0 on FAT16)
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

def create_fat32_image(output_path: str, files: list):
    """
    Create a FAT16 disk image at *output_path* containing *files*.

    (Function name kept for backwards compatibility with callers.)

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
    root_dir_sectors = (ROOT_ENTRIES * 32 + SECTOR_SIZE - 1) // SECTOR_SIZE

    total_bytes = sum(len(d) for d in file_data.values())
    needed      = int(total_bytes * 1.2) + 1 * 1024 * 1024   # 1 MB FAT+dir overhead
    min_sectors = max(2048, (needed + SECTOR_SIZE - 1) // SECTOR_SIZE)
    sector_count = ((min_sectors + 8191) // 8192) * 8192      # align to 4 MB

    # FAT16 entries are 2 bytes.  Iterate to converge on a FAT size large enough
    # to map every data cluster (FAT size depends on cluster count, which depends
    # on FAT size).  A single pass over-estimates safely.
    def fat_sectors_for(data_sectors_guess: int) -> int:
        clusters = data_sectors_guess + 2  # +2 for reserved FAT entries 0,1
        fat_bytes = clusters * 2
        fs = (fat_bytes + SECTOR_SIZE - 1) // SECTOR_SIZE
        return ((fs + 31) // 32) * 32                         # align to 16 KB

    fat_sectors = fat_sectors_for(sector_count)
    data_start  = RESERVED + FATS * fat_sectors + root_dir_sectors
    data_clusters = sector_count - data_start

    # Guard: keep the cluster count inside the FAT16 window.
    if data_clusters < FAT16_MIN_CLUSTERS:
        # Grow the image so the data region clears the FAT12/FAT16 boundary.
        sector_count = data_start + FAT16_MIN_CLUSTERS + 16
        sector_count = ((sector_count + 8191) // 8192) * 8192
        data_clusters = sector_count - data_start
    if data_clusters > FAT16_MAX_CLUSTERS:
        raise ValueError(
            f"image needs {data_clusters} clusters (> FAT16 max {FAT16_MAX_CLUSTERS}); "
            f"reduce embedded file size or switch to a larger cluster size"
        )

    # ── 3. Cluster allocator (FAT16: 2-byte entries) ─────────────────────────
    fat = bytearray(fat_sectors * SECTOR_SIZE)
    struct.pack_into('<H', fat, 0, 0xFFF8)       # entry 0 (media descriptor)
    struct.pack_into('<H', fat, 2, 0xFFFF)       # entry 1 (EOC)

    next_cluster = [2]   # mutable int via list

    def alloc_chain(num_clusters: int) -> int:
        """Allocate a chain of *num_clusters* clusters; return start cluster."""
        if num_clusters == 0:
            return 0
        start = next_cluster[0]
        for i in range(num_clusters):
            c = next_cluster[0]
            next_cluster[0] += 1
            nxt = next_cluster[0] if i < num_clusters - 1 else EOC16
            struct.pack_into('<H', fat, c * 2, nxt)
        return start

    def cluster_offset(cluster: int) -> int:
        """Byte offset of *cluster* in the image."""
        return (data_start + (cluster - 2)) * SECTOR_SIZE

    # ── 4. Build directory tree ──────────────────────────────────────────────
    # dir_files[dir_path] → list of (filename, cluster, size, attr); '' = root
    dir_files = defaultdict(list)

    dirs = {''}    # root always exists
    for dst in file_data:
        parts = dst.split('/')
        for i in range(len(parts) - 1):
            dirs.add('/'.join(parts[:i+1]))

    # Root lives in the fixed root-dir region (no cluster).  Subdirectories get
    # one cluster each in the data region.
    dir_cluster = {'': 0}
    for d in sorted(dirs):
        if d == '':
            continue
        dir_cluster[d] = alloc_chain(1)

    # Register subdirectories in their parent dirs.
    for d in sorted(dirs):
        if d == '':
            continue
        parent = d.rsplit('/', 1)[0] if '/' in d else ''
        name   = d.rsplit('/', 1)[-1]
        dir_files[parent].append((name, dir_cluster[d], 0, ATTR_DIRECTORY))

    # Allocate clusters for files and register them in their parent dirs.
    file_cluster = {}
    for dst, data in file_data.items():
        num_c = max(1, (len(data) + CLUSTER_SIZE - 1) // CLUSTER_SIZE)
        fc    = alloc_chain(num_c)
        file_cluster[dst] = fc
        parent = dst.rsplit('/', 1)[0] if '/' in dst else ''
        name   = dst.rsplit('/', 1)[-1]
        dir_files[parent].append((name, fc, len(data), ATTR_ARCHIVE))

    root_dir_start = RESERVED + FATS * fat_sectors  # sector of fixed root dir

    # ── 5. Write image ───────────────────────────────────────────────────────
    with open(output_path, 'wb+') as img:
        # a) Zero-fill
        img.seek(sector_count * SECTOR_SIZE - 1)
        img.write(b'\x00')
        img.seek(0)

        # b) Boot sector (FAT16 BPB)
        img.write(b'\xEB\x3C\x90' + b'MSWIN4.1')
        img.write(struct.pack('<H', SECTOR_SIZE))      # bytes/sector
        img.write(struct.pack('<B', 1))                # sectors/cluster
        img.write(struct.pack('<H', RESERVED))         # reserved sectors
        img.write(struct.pack('<B', FATS))             # FAT count
        img.write(struct.pack('<H', ROOT_ENTRIES))     # root entry count (FAT16 != 0)
        if sector_count < 0x10000:
            img.write(struct.pack('<H', sector_count)) # total sectors 16-bit
        else:
            img.write(struct.pack('<H', 0))            # use 32-bit field below
        img.write(b'\xF8')                             # media descriptor
        img.write(struct.pack('<H', fat_sectors))      # FAT size 16-bit (FAT16)
        img.write(struct.pack('<H', 63))               # sectors/track
        img.write(struct.pack('<H', 255))              # heads
        img.write(struct.pack('<I', 0))                # hidden sectors
        if sector_count < 0x10000:
            img.write(struct.pack('<I', 0))            # total sectors 32-bit (unused)
        else:
            img.write(struct.pack('<I', sector_count)) # total sectors 32-bit
        # FAT16 extended BPB (starts at offset 36)
        img.write(struct.pack('<B', 0x80))             # BS_DrvNum
        img.write(struct.pack('<B', 0))                # BS_Reserved1
        img.write(struct.pack('<B', 0x29))             # BS_BootSig
        img.write(struct.pack('<I', 0x12345678))       # BS_VolID
        img.write(b'ViCell       ')                      # BS_VolLab (11 bytes)
        img.write(b'FAT16   ')                         # BS_FilSysType (8 bytes)
        img.seek(510)
        img.write(b'\x55\xAA')

        # c) FAT tables
        img.seek(RESERVED * SECTOR_SIZE)
        img.write(fat)
        img.seek((RESERVED + fat_sectors) * SECTOR_SIZE)
        img.write(fat)

        # d) Directory entries
        for dir_path, entries in dir_files.items():
            if dir_path == '':
                off = root_dir_start * SECTOR_SIZE
            else:
                off = cluster_offset(dir_cluster[dir_path])
            img.seek(off)

            # . and .. for non-root directories
            if dir_path != '':
                parent_path = dir_path.rsplit('/', 1)[0] if '/' in dir_path else ''
                img.write(dot_entries(dir_cluster[dir_path], dir_cluster[parent_path]))

            for name, cluster, size, attr in entries:
                img.write(dir_entry(make_83_name(name), cluster, size, attr))

        # e) File data
        for dst, data in file_data.items():
            off = cluster_offset(file_cluster[dst])
            img.seek(off)
            img.write(data)

    ndirs  = len(dirs) - 1  # exclude root
    nfiles = len(file_data)
    print(f"Created FAT16 image at {output_path}: "
          f"{nfiles} file(s), {ndirs} director(ies), {data_clusters} clusters.")


if __name__ == '__main__':
    if len(sys.argv) < 2:
        print('Usage: mkfat32.py <output.img> [<src> <dst>] ...')
        sys.exit(1)
    out   = sys.argv[1]
    args  = sys.argv[2:]
    pairs = [(args[i], args[i+1]) for i in range(0, len(args) - 1, 2)]
    create_fat32_image(out, pairs)
