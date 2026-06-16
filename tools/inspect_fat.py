"""inspect_fat.py - inspect FAT16 image directory structure."""
import struct, os, sys

path = sys.argv[1] if len(sys.argv) > 1 else 'kernel/src/embedded-aarch64/kernel_fs.img'
print('Checking:', path)
sz = os.path.getsize(path)
print('Size:', sz, 'bytes (%d KB)' % (sz // 1024))

with open(path, 'rb') as f:
    data = f.read()

bs = data[:512]
print('Sig:', bs[510:512].hex(), '(should be 55aa)')
print('FS type:', bs[54:62])
bps = struct.unpack_from('<H', bs, 11)[0]
spc = bs[13]
res_s = struct.unpack_from('<H', bs, 14)[0]
fc = bs[16]
re = struct.unpack_from('<H', bs, 17)[0]
fat_sz16 = struct.unpack_from('<H', bs, 22)[0]
fat_sz32 = struct.unpack_from('<I', bs, 36)[0]
fat_sz = fat_sz16 if fat_sz16 else fat_sz32
print('BPS=%d SPC=%d RES=%d FAT_CNT=%d ROOT_ENT=%d FAT_SZ=%d' % (bps, spc, res_s, fc, re, fat_sz))

root_off = (res_s + fc * fat_sz) * bps
data_off = root_off + re * 32
print('Root dir at %d, Data area at %d' % (root_off, data_off))


def parse_dir(data, offset, label):
    print('  --- %s ---' % label)
    pending_lfn = []
    for i in range(512):
        e = data[offset + i*32: offset + (i+1)*32]
        if len(e) < 32 or e[0] == 0:
            break
        if e[0] == 0xE5:
            continue
        attr = e[11]
        if attr == 0x0F:
            # LFN entry - collect name fragments
            seq = e[0] & 0x3F
            n1 = e[1:11].decode('utf-16-le', errors='replace').rstrip('￿')
            n2 = e[14:26].decode('utf-16-le', errors='replace').rstrip('￿')
            n3 = e[28:32].decode('utf-16-le', errors='replace').rstrip('￿')
            frag = (n1 + n2 + n3).rstrip('\x00')
            print('    LFN[%d] %r' % (seq, frag))
            pending_lfn.append((seq, frag))
            continue
        name = e[:8].rstrip(b' ')
        ext = e[8:11].rstrip(b' ')
        size = struct.unpack_from('<I', e, 28)[0]
        clus = struct.unpack_from('<H', e, 26)[0]
        full = name + (b'.' + ext if ext else b'')
        # Reconstruct LFN if pending
        if pending_lfn:
            sorted_frags = sorted(pending_lfn, key=lambda x: x[0])
            lfn_full = ''.join(f for _, f in sorted_frags)
            print('    SFN %-12s  -> LFN %r  attr=%02x clus=%d sz=%d' % (
                full.decode(errors='replace'), lfn_full, attr, clus, size))
            pending_lfn = []
        else:
            print('    SFN %-20s attr=%02x clus=%d sz=%d' % (full.decode(errors='replace'), attr, clus, size))


parse_dir(data, root_off, 'root')

# Find BIN dir cluster
for i in range(re):
    e = data[root_off + i*32: root_off + (i+1)*32]
    if e[0] == 0:
        break
    if e[0] == 0xE5:
        continue
    if e[11] == 0x0F:
        continue
    name = e[:8].rstrip(b' ')
    attr = e[11]
    if (attr & 0x10) and name not in (b'.', b'..'):
        clus = struct.unpack_from('<H', e, 26)[0]
        bin_off = data_off + (clus - 2) * spc * bps
        print('BIN dir (SFN=%r) at cluster %d, offset %d' % (name.decode(errors='replace'), clus, bin_off))
        parse_dir(data, bin_off, '/bin')
        break
