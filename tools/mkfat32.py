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
ATTR_LONG_NAME = 0x0F             # LFN entry attribute
EOC16          = 0xFFFF           # end-of-chain marker (FAT16)
DATE_DEFAULT   = 0x5700           # 2026-11-16 encoded (year-1980=46, mon=11, day=16)
TIME_DEFAULT   = 0x8C00           # 17:32:00

# FAT16 valid cluster-count window (outside → FAT12 or FAT32, which we must avoid).
FAT16_MIN_CLUSTERS = 4085
FAT16_MAX_CLUSTERS = 65524

# Characters valid in an 8.3 SFN base/ext field (per FAT spec §6.1).
# Dash (-) is included; the spec only prohibits specific control/punctuation bytes.
VALID_SFN_CHARS = set(
    'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
    '!#$%&\'()-@^_`{}~'
)


# ── Directory-entry helpers ───────────────────────────────────────────────────

def make_83_name(name: str) -> bytes:
    """Convert a simple name (no path) to 8.3 format, padded with spaces.

    Used only for names that fit 8.3 exactly.  For long names use
    make_sfn_for_lfn() + lfn_entries_for() instead.
    """
    name = name.upper()
    if '.' in name:
        base, ext = name.rsplit('.', 1)
    else:
        base, ext = name, ''
    return (base[:8].ljust(8) + ext[:3].ljust(3)).encode('ascii')


def needs_lfn(name: str) -> bool:
    """True when *name* cannot be stored as a plain 8.3 SFN.

    A name needs LFN when:
    - the base (before the last dot) is longer than 8 characters, OR
    - the extension is longer than 3 characters, OR
    - the original case differs from the uppercased form (preserve case via LFN).
    """
    upper = name.upper()
    dot_idx = upper.rfind('.')
    if dot_idx < 0:
        base, ext = upper, ''
    else:
        base, ext = upper[:dot_idx], upper[dot_idx + 1:]
    if len(base) > 8 or len(ext) > 3:
        return True
    if name != upper:   # original had lowercase → preserve it via LFN
        return True
    return False


def sfn_checksum(sfn11: bytes) -> int:
    """Compute the LFN checksum of an 11-byte 8.3 SFN field.

    The checksum is rotated right 1 bit and added for each byte (FAT spec §7).
    """
    assert len(sfn11) == 11
    csum = 0
    for b in sfn11:
        csum = (((csum & 0x01) << 7) | (csum >> 1)) + b
        csum &= 0xFF
    return csum


def make_sfn_for_lfn(name: str, suffix_num: int = 1) -> bytes:
    """Build a ~N-style 11-byte SFN for use alongside an LFN entry.

    Algorithm mirrors Windows short-name generation:
      1. uppercase, keep only VALID_SFN_CHARS in each part
      2. take first (8 − len("~N")) chars of the base
      3. append the "~N" suffix

    *suffix_num* is the collision counter (default 1).
    """
    upper = name.upper()
    dot_idx = upper.rfind('.')
    if dot_idx < 0:
        base_raw, ext_raw = upper, ''
    else:
        base_raw, ext_raw = upper[:dot_idx], upper[dot_idx + 1:]

    base_filt = ''.join(c for c in base_raw if c in VALID_SFN_CHARS)
    ext_filt  = ''.join(c for c in ext_raw  if c in VALID_SFN_CHARS)[:3]

    suffix    = '~%d' % suffix_num          # e.g. "~1"
    avail     = 8 - len(suffix)
    sfn_base  = (base_filt[:avail] + suffix).ljust(8)
    sfn_ext   = ext_filt.ljust(3)
    return (sfn_base + sfn_ext).encode('ascii')


def lfn_entries_for(name: str, sfn11: bytes) -> list:
    """Return a list of 32-byte LFN directory entries for *name*.

    The entries are returned in **directory order**: the entry with the highest
    (last) sequence number — containing the end of the filename — comes first,
    and the entry with sequence number 1 — containing the start — comes last,
    just before the associated SFN entry.

    *sfn11* must be the 11-byte SFN that immediately follows these entries;
    its checksum is embedded in every LFN entry.
    """
    checksum   = sfn_checksum(sfn11)
    ucs2       = name.encode('utf-16-le')
    num_entries = (len(name) + 1 + 12) // 13   # ceil((len+null) / 13 chars)

    # Build the padded UCS-2 sequence: name + null-terminator + 0xFFFF fill.
    full = bytearray(ucs2) + b'\x00\x00'
    while len(full) < num_entries * 26:
        full += b'\xFF\xFF'

    entries = []
    for i in range(num_entries):
        seq = i + 1
        if seq == num_entries:
            seq |= 0x40         # mark as last (= highest sequence number)

        chunk = bytes(full[i * 26: (i + 1) * 26])
        entry = (
            bytes([seq]) +
            chunk[0:10] +       # name chars 1-5  (UCS-2 LE)
            bytes([ATTR_LONG_NAME, 0x00, checksum]) +
            chunk[10:22] +      # name chars 6-11 (UCS-2 LE)
            bytes([0x00, 0x00]) +   # FstClusLO = 0 (required by spec)
            chunk[22:26]        # name chars 12-13 (UCS-2 LE)
        )
        assert len(entry) == 32
        entries.append(entry)

    # Reverse so highest seq# (last LFN chunk) comes first in the directory.
    return list(reversed(entries))


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

    # Auto-select sectors/cluster: start at 1, double until data fits in FAT16
    # window (<=65524 clusters).  Valid values per FAT spec: 1,2,4,8,16,32,64,128.
    sectors_per_cluster = 1
    while True:
        cluster_size = SECTOR_SIZE * sectors_per_cluster

        # FAT16 entries are 2 bytes.  Iterate to converge on a FAT size large
        # enough to map every data cluster.
        def fat_sectors_for(data_sectors_guess: int) -> int:
            clusters = (data_sectors_guess // sectors_per_cluster) + 2
            fat_bytes = clusters * 2
            fs = (fat_bytes + SECTOR_SIZE - 1) // SECTOR_SIZE
            return ((fs + 31) // 32) * 32                     # align to 16 KB

        fat_sectors = fat_sectors_for(sector_count)
        # data_start must match the BPB-implied value so the fatfs crate and this
        # script agree on where cluster 2 begins.  FAT16 does not require the data
        # area to start on a cluster boundary — keeping it equal to the BPB formula
        # (reserved + fats*fat_size + root_dir_sectors) is the only correct choice.
        data_start  = RESERVED + FATS * fat_sectors + root_dir_sectors
        data_sectors = sector_count - data_start
        data_clusters = data_sectors // sectors_per_cluster

        # Guard: keep the cluster count inside the FAT16 window.
        if data_clusters < FAT16_MIN_CLUSTERS:
            sector_count = data_start + FAT16_MIN_CLUSTERS * sectors_per_cluster + 16
            sector_count = ((sector_count + 8191) // 8192) * 8192
            data_sectors = sector_count - data_start
            data_clusters = data_sectors // sectors_per_cluster

        if data_clusters <= FAT16_MAX_CLUSTERS:
            break  # geometry fits

        # Too many clusters — double cluster size and retry.
        sectors_per_cluster *= 2
        if sectors_per_cluster > 128:
            raise ValueError(
                f"image too large even at 128 sectors/cluster "
                f"({total_bytes // (1024*1024)} MB); reduce embedded file size"
            )
        # Recalculate sector_count at the new cluster granularity.
        sector_count = ((min_sectors + 8191) // 8192) * 8192

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
        return (data_start + (cluster - 2) * sectors_per_cluster) * SECTOR_SIZE

    # ── 4. Build directory tree ──────────────────────────────────────────────
    # dir_files[dir_path] → list of (filename, cluster, size, attr); '' = root
    dir_files = defaultdict(list)

    dirs = {''}    # root always exists
    for dst in file_data:
        parts = dst.split('/')
        for i in range(len(parts) - 1):
            dirs.add('/'.join(parts[:i+1]))

    def entry_slots(name: str) -> int:
        """32-byte directory slots consumed by this name (LFN entries + 1 SFN)."""
        if not needs_lfn(name):
            return 1
        return (len(name) + 1 + 12) // 13 + 1

    def dir_slot_count(d: str) -> int:
        """Total 32-byte slots needed for directory d."""
        count = 2 if d != '' else 0   # . and .. for non-root dirs
        for sub in sorted(dirs):
            if sub == '': continue
            p = sub.rsplit('/', 1)[0] if '/' in sub else ''
            if p == d:
                count += entry_slots(sub.rsplit('/', 1)[-1])
        for dst in file_data:
            p = dst.rsplit('/', 1)[0] if '/' in dst else ''
            if p == d:
                count += entry_slots(dst.rsplit('/', 1)[-1])
        return count

    # Root lives in the fixed root-dir region (no cluster).  Subdirectories get
    # enough clusters to hold all their directory entries.
    dir_cluster = {'': 0}
    for d in sorted(dirs):
        if d == '':
            continue
        slots = dir_slot_count(d)
        n_clusters = max(1, (slots * 32 + cluster_size - 1) // cluster_size)
        dir_cluster[d] = alloc_chain(n_clusters)

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
        num_c = max(1, (len(data) + cluster_size - 1) // cluster_size)
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
        img.write(struct.pack('<B', sectors_per_cluster)) # sectors/cluster
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
        # Track ~N suffix counters per 6-char SFN prefix to avoid collisions.
        sfn_suffix_counter: dict = {}

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
                if needs_lfn(name):
                    # Generate a unique ~N SFN for this long filename.
                    upper = name.upper()
                    dot_idx = upper.rfind('.')
                    base_raw = upper[:dot_idx] if dot_idx >= 0 else upper
                    base_filt = ''.join(c for c in base_raw if c in VALID_SFN_CHARS)
                    prefix_key = (dir_path, base_filt[:6])
                    n = sfn_suffix_counter.get(prefix_key, 0) + 1
                    sfn_suffix_counter[prefix_key] = n
                    sfn11 = make_sfn_for_lfn(name, n)
                    for lfn_e in lfn_entries_for(name, sfn11):
                        img.write(lfn_e)
                    img.write(dir_entry(sfn11, cluster, size, attr))
                else:
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
