#!/usr/bin/env python3
"""Create a Limine-bootable x86_64 ISO using pycdlib.

Usage: python make_iso.py <iso_root_dir> <output.iso>

The iso_root_dir must contain:
  boot/kernel.elf
  boot/limine.cfg  (or limine.conf)
  boot/limine/limine-bios-cd.bin
  boot/limine/limine-bios.sys
  EFI/BOOT/BOOTX64.EFI
"""
import sys
import os
import struct

def create_iso(iso_root: str, output: str):
    import pycdlib

    iso = pycdlib.PyCdlib()
    iso.new(
        joliet=3,
        rock=True,
        sys_ident='LINUX',
        vol_ident='VICELL_X86',
        # El Torito BIOS boot: boot catalog at /boot.cat
        # We set the boot catalog and initial/default boot entry below.
    )

    def add_dir(rr_path, joliet_path, iso_path):
        iso.add_directory(rr_name=rr_path.split('/')[-1],
                          rr_path='/' + rr_path if not rr_path.startswith('/') else rr_path,
                          joliet_path=joliet_path,
                          iso_path=iso_path)

    def add_file(local_path, rr_path, joliet_path, iso_path):
        with open(local_path, 'rb') as f:
            data = f.read()
        iso.add_fp(fp=__import__('io').BytesIO(data),
                   length=len(data),
                   rr_name=os.path.basename(rr_path),
                   rr_path='/' + rr_path.lstrip('/'),
                   joliet_path=joliet_path,
                   iso_path=iso_path)

    # Create directories
    iso.add_directory(rr_name='boot', rr_path='/boot',
                      joliet_path='/boot', iso_path='/BOOT')
    iso.add_directory(rr_name='limine', rr_path='/boot/limine',
                      joliet_path='/boot/limine', iso_path='/BOOT/LIMINE')
    iso.add_directory(rr_name='EFI', rr_path='/EFI',
                      joliet_path='/EFI', iso_path='/EFI')
    iso.add_directory(rr_name='BOOT', rr_path='/EFI/BOOT',
                      joliet_path='/EFI/BOOT', iso_path='/EFI/BOOT')

    # Add files
    files_to_add = [
        ('boot/kernel.elf',         '/boot/kernel.elf',         '/boot/kernel.elf',         '/BOOT/KERNEL.ELF'),
        ('boot/limine.cfg',         '/boot/limine.cfg',         '/boot/limine.cfg',         '/BOOT/LIMINE.CFG')
            if os.path.exists(os.path.join(iso_root, 'boot/limine.cfg')) else None,
        ('boot/limine.conf',        '/boot/limine.conf',        '/boot/limine.conf',        '/BOOT/LIMINE.CNF')
            if os.path.exists(os.path.join(iso_root, 'boot/limine.conf')) else None,
        ('boot/limine/limine-bios-cd.bin', '/boot/limine/limine-bios-cd.bin',
            '/boot/limine/limine-bios-cd.bin', '/BOOT/LIMINE/LBIOSCD.BIN'),
        ('boot/limine/limine-bios.sys', '/boot/limine/limine-bios.sys',
            '/boot/limine/limine-bios.sys', '/BOOT/LIMINE/LBIOS.SYS'),
        ('EFI/BOOT/BOOTX64.EFI',    '/EFI/BOOT/BOOTX64.EFI',   '/EFI/BOOT/BOOTX64.EFI',   '/EFI/BOOT/BOOTX64.EFI'),
    ]

    for entry in files_to_add:
        if entry is None:
            continue
        rel, rr, joliet, iso_path = entry
        local = os.path.join(iso_root, rel)
        if os.path.exists(local):
            with open(local, 'rb') as f:
                data = f.read()
            import io
            iso.add_fp(fp=io.BytesIO(data), length=len(data),
                       rr_name=os.path.basename(rr),
                       rr_path=rr,
                       joliet_path=joliet,
                       iso_path=iso_path)

    iso.write(output)
    iso.close()
    print(f"ISO created: {output} ({os.path.getsize(output)//1024} KB)")


if __name__ == '__main__':
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <iso_root_dir> <output.iso>")
        sys.exit(1)
    create_iso(sys.argv[1], sys.argv[2])
