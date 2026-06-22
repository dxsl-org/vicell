"""Read and print the Cellos cell bootstrap table from disk_v3.img."""
import struct, sys, os

SECTOR_SIZE       = 512
CELL_TABLE_BASE_LBA = 526_336
CELL_TABLE_MAGIC  = 0x5649_4F53_5F43_454C
CELL_PATH_LEN     = 64

disk_img = sys.argv[1] if len(sys.argv) > 1 else "disk_v3.img"
with open(disk_img, "rb") as f:
    f.seek(CELL_TABLE_BASE_LBA * SECTOR_SIZE)
    hdr = f.read(512)
    magic, count = struct.unpack_from("<QI", hdr)
    if magic != CELL_TABLE_MAGIC:
        print(f"ERROR: wrong magic {magic:#x}, expected {CELL_TABLE_MAGIC:#x}")
        sys.exit(1)
    print(f"Cell table: {count} entries")
    for _ in range(count):
        entry = f.read(512)
        path_raw = entry[:CELL_PATH_LEN]
        path = path_raw.split(b"\x00", 1)[0].decode("utf-8")
        data_lba, data_size = struct.unpack_from("<QQ", entry, CELL_PATH_LEN)
        print(f"  {path:<40}  LBA={data_lba}  size={data_size}")
