# Create proper 40MB FAT32 disk image to satisfy cluster count > 65525
# 8MB was too small for FAT32 spec (forced FAT16/12)

import struct
import os

# 40MB is enough to have > 65525 clusters with 1 sector/cluster
DISK_SIZE_MB = 40
DISK_SIZE_BYTES = DISK_SIZE_MB * 1024 * 1024
SECTOR_SIZE = 512
TOTAL_SECTORS = DISK_SIZE_BYTES // SECTOR_SIZE  # 81920

# Parameters for valid FAT32
SECTORS_PER_CLUSTER = 1
RESERVED_SECTORS = 32
FAT_COUNT = 2

# Calculate Sectors Per FAT
# Formula: (TotalSectors - Reserved) / (SectorsPerCluster * 128 + FATCount) 
# Note: 128 entries per section (512/4)
# (81920 - 32) / (1 * 128 + 2) = 81888 / 130 = ~629.9
SECTORS_PER_FAT = 632  # Round up for safety

print(f"Creating {DISK_SIZE_MB}MB FAT32 disk image (Spec Compliant)...")

# Create empty disk
disk = bytearray(DISK_SIZE_BYTES)

# Create boot sector
boot = bytearray(512)
boot[0:3] = bytes([0xEB, 0x58, 0x90])  # Jump instruction
boot[3:11] = b"MSWIN4.1"  # OEM name
boot[11:13] = struct.pack('<H', SECTOR_SIZE)  # Bytes per sector
boot[13] = SECTORS_PER_CLUSTER  # Sectors per cluster
boot[14:16] = struct.pack('<H', RESERVED_SECTORS)  # Reserved sectors
boot[16] = FAT_COUNT  # Number of FATs
boot[17:19] = struct.pack('<H', 0)  # Root entries (0 for FAT32)
boot[19:21] = struct.pack('<H', 0)  # Total sectors 16-bit (0 for FAT32)
boot[21] = 0xF8  # Media descriptor
boot[22:24] = struct.pack('<H', 0)  # Sectors per FAT (0 for FAT32, use 32-bit)
boot[24:26] = struct.pack('<H', 63)  # Sectors per track
boot[26:28] = struct.pack('<H', 255)  # Number of heads
boot[28:32] = struct.pack('<I', 0)  # Hidden sectors
boot[32:36] = struct.pack('<I', TOTAL_SECTORS)  # Total sectors 32-bit

# FAT32 extended BPB
boot[36:40] = struct.pack('<I', SECTORS_PER_FAT)  # Sectors per FAT
boot[40:42] = struct.pack('<H', 0)  # Flags
boot[42:44] = struct.pack('<H', 0)  # Version
boot[44:48] = struct.pack('<I', 2)  # Root cluster
boot[48:50] = struct.pack('<H', 1)  # FSInfo sector
boot[50:52] = struct.pack('<H', 6)  # Backup boot sector
boot[52:64] = bytes(12)  # Reserved
boot[64] = 0x80  # Drive number
boot[65] = 0  # Reserved
boot[66] = 0x29  # Extended boot signature
boot[67:71] = struct.pack('<I', 0x12345678)  # Volume ID
boot[71:82] = b"Cellos_BOOT  "  # Volume label
boot[82:90] = b"FAT32   "  # Filesystem type
boot[510:512] = bytes([0x55, 0xAA])  # Boot signature

disk[0:512] = boot
print("  ✓ Created boot sector")

# Create FSInfo sector
fsinfo = bytearray(512)
fsinfo[0:4] = struct.pack('<I', 0x41615252)  # Lead signature
fsinfo[484:488] = struct.pack('<I', 0x61417272)  # Struct signature
fsinfo[488:492] = struct.pack('<I', 0xFFFFFFFF)  # Free cluster count
fsinfo[492:496] = struct.pack('<I', 3)  # Next free cluster (start at 3)
fsinfo[508:512] = struct.pack('<I', 0xAA550000)  # Trail signature

disk[512:1024] = fsinfo
print("  ✓ Created FSInfo sector")

# Initialize FAT tables
# FAT entries are 32-bit.
# Entry 0: Media Check (0x0FFFFF00 | 0xF8) -> 0x0FFFFFF8
# Entry 1: EOC/Hard Mask (0x0FFFFFFF)
# Entry 2: Root Directory EOC (0x0FFFFFFF) - marks end of root chain

fat1_offset = RESERVED_SECTORS * SECTOR_SIZE
fat1 = bytearray(SECTORS_PER_FAT * SECTOR_SIZE)

# Set entries 0, 1, 2
struct.pack_into('<I', fat1, 0, 0x0FFFFFF8)
struct.pack_into('<I', fat1, 4, 0x0FFFFFFF)
struct.pack_into('<I', fat1, 8, 0x0FFFFFFF)

disk[fat1_offset:fat1_offset + len(fat1)] = fat1
print("  ✓ Created FAT1")

# Copy to FAT2
fat2_offset = fat1_offset + (SECTORS_PER_FAT * SECTOR_SIZE)
disk[fat2_offset:fat2_offset + len(fat1)] = fat1
print("  ✓ Created FAT2")

# Calculate Data Start
data_start_sector = RESERVED_SECTORS + (FAT_COUNT * SECTORS_PER_FAT)
print(f"  ✓ Data region starts at sector {data_start_sector}")

# Helper to write file
def write_file(name_str, cluster_start, file_path):
    if not os.path.exists(file_path):
        print(f"  ✗ Warning: {file_path} not found")
        return 0, 0
        
    with open(file_path, "rb") as f:
        content = f.read()
        
    size = len(content)
    # Calculate sectors needed (round up)
    sectors_needed = (size + SECTOR_SIZE - 1) // SECTOR_SIZE
    
    # Write content to data region
    # Cluster N is at: data_start_sector + (N - 2) * SECTORS_PER_CLUSTER
    # Since SECTORS_PER_CLUSTER = 1, Cluster N is at data_start_sector + N - 2
    offset = (data_start_sector + cluster_start - 2) * SECTOR_SIZE
    disk[offset:offset + size] = content
    
    # Update FAT table chain
    # Simple contiguous chain
    for i in range(sectors_needed):
        current_cluster = cluster_start + i
        next_cluster = cluster_start + i + 1 if i < sectors_needed - 1 else 0x0FFFFFFF
        struct.pack_into('<I', fat1, current_cluster * 4, next_cluster)
        
    # Copy FAT1 changes to FAT2
    start_fat_offset = cluster_start * 4
    end_fat_offset = (cluster_start + sectors_needed) * 4
    disk[fat1_offset + start_fat_offset : fat1_offset + end_fat_offset] = fat1[start_fat_offset : end_fat_offset]
    
    fat2_offset = fat1_offset + (SECTORS_PER_FAT * SECTOR_SIZE)
    disk[fat2_offset + start_fat_offset : fat2_offset + end_fat_offset] = fat1[start_fat_offset : end_fat_offset]

    print(f"  ✓ Injected {name_str} at Cluster {cluster_start}, Size {size} bytes")
    return size, unique_clusters_used(sectors_needed)

def unique_clusters_used(sectors):
    return sectors # Since 1 sector/cluster

# Inject INIT
# 8.3 Name: "INIT    " + "   "
init_name = b"INIT       "
init_size, init_clusters = write_file("INIT", 3, "target/riscv64gc-unknown-none-elf/release/app-init")

# Inject SHELL
shell_name = b"SHELL      "
shell_cluster_start = 3 + init_clusters
shell_size, shell_clusters = write_file("SHELL", shell_cluster_start, "target/riscv64gc-unknown-none-elf/release/app-shell")

# Create Directory Entries in Root (Cluster 2)
# Root Dir starts at data_start_sector
root_offset = (data_start_sector + 2 - 2) * SECTOR_SIZE

# Entry 1: INIT
if init_size > 0:
    # Name (11)
    disk[root_offset:root_offset+11] = init_name
    # Attr (0x20 Archive)
    disk[root_offset+11] = 0x20
    # High Cluster
    struct.pack_into('<H', disk, root_offset+20, 3 >> 16)
    # Low Cluster
    struct.pack_into('<H', disk, root_offset+26, 3 & 0xFFFF)
    # Size
    struct.pack_into('<I', disk, root_offset+28, init_size)
    root_offset += 32

# Entry 2: SHELL
if shell_size > 0:
    disk[root_offset:root_offset+11] = shell_name
    disk[root_offset+11] = 0x20
    struct.pack_into('<H', disk, root_offset+20, shell_cluster_start >> 16)
    struct.pack_into('<H', disk, root_offset+26, shell_cluster_start & 0xFFFF)
    struct.pack_into('<I', disk, root_offset+28, shell_size)
    root_offset += 32

print(f"  ✓ Updated Root Directory")

# Write output
output_file = "disk_40mb.img"

with open(output_file, "wb") as f:
    f.write(disk)

print(f"\n✓ Created {output_file} ({DISK_SIZE_MB}MB)")
print(f"  Total Sectors: {TOTAL_SECTORS}")
print(f"  Sectors/FAT: {SECTORS_PER_FAT}")
