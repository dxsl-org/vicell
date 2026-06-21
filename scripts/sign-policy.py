#!/usr/bin/env python3
"""Build + Ed25519-sign a ViCell operator policy blob (roadmap §G.2 P5b).

Blob format (little-endian) — MUST match kernel/src/policy.rs::parse:
    magic u32 "VPOL" | version u8=1 | flags u8 | entry_count u16
    per entry: path_len u8, path bytes,
               block_io u8, network u8, spawn u8, hyp u8, mmio_devices u8, block_regions u8
    + Ed25519 signature [u8;64] over all preceding bytes

The DEV key is derived from a fixed seed so it is reproducible (it is a *dev*
key, gated behind the kernel `dev-policy-key` feature, never shipped in release).
Production signing supplies a real private key via --seed-hex / a KMS, never this
hardcoded dev seed.

Usage:
    python scripts/sign-policy.py --emit-rust   # print dev pubkey + signed blob as Rust literals
    python scripts/sign-policy.py --out POLICY.BIN   # write the signed blob for baking into VIFS1
"""
import argparse
import struct
import sys

try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives import serialization
except ImportError:
    sys.exit("error: pip install cryptography")

MAGIC = b"VPOL"
VERSION = 1
DEV_SEED = bytes([0x42] * 32)  # fixed dev seed → deterministic dev keypair

# Dev policy: mirrors the current first-party grants so a baked blob is non-breaking.
# (path, block_io, network, spawn, hyp, mmio_devices, block_regions)
# mmio_devices bits: DEV_UART=1, DEV_GPIO=2.  block_regions: P1=1, P4=2, SRV=4 (0b111=all).
DEV_POLICY = [
    ("/bin/vfs",   1, 0, 0, 0, 0, 0b111),
    ("/bin/net",   0, 1, 0, 0, 0, 0),
    ("/bin/shell", 0, 0, 1, 0, 0, 0),
    ("/bin/init",  1, 1, 1, 0, 3, 0b111),  # root authority (informational; init is exempt in-kernel)
]


def build_body(entries):
    out = bytearray()
    out += MAGIC
    out += struct.pack("<BBH", VERSION, 0, len(entries))
    for (path, bio, net, spawn, hyp, mmio, regions) in entries:
        pb = path.encode("ascii")
        if len(pb) > 255:
            sys.exit(f"path too long: {path}")
        out.append(len(pb))
        out += pb
        out += bytes([bio, net, spawn, hyp, mmio, regions])
    return bytes(out)


def sign(body, seed=DEV_SEED):
    priv = Ed25519PrivateKey.from_private_bytes(seed)
    pub = priv.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )
    sig = priv.sign(body)
    return pub, sig


def rust_array(name, data):
    body = ", ".join(f"0x{b:02x}" for b in data)
    return f"pub const {name}: [u8; {len(data)}] = [{body}];"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--emit-rust", action="store_true", help="print dev pubkey + signed blob as Rust literals")
    ap.add_argument("--out", help="write the signed blob to this file (for baking into VIFS1 as /POLICY.BIN)")
    args = ap.parse_args()

    body = build_body(DEV_POLICY)
    pub, sig = sign(body)
    blob = body + sig

    if args.out:
        with open(args.out, "wb") as f:
            f.write(blob)
        print(f"wrote {len(blob)} bytes to {args.out}", file=sys.stderr)

    if args.emit_rust or not args.out:
        print(rust_array("DEV_FLEET_PUBKEY", pub))
        print(rust_array("DEV_POLICY_BLOB", blob))
        print(f"// blob = {len(blob)} bytes ({len(body)} body + 64 sig), {len(DEV_POLICY)} entries", file=sys.stderr)


if __name__ == "__main__":
    main()
