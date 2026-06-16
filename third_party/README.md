# Third-party vendored crates

Vendored forks of external crates adapted for ViCell's no_std/no_main bare-metal target.
These are NOT git submodules — full source snapshots, owned by this repo.

## redoxfs

**Upstream**: https://github.com/redox-os/redoxfs  
**Version**: 0.9.0  
**License**: MIT  
**Purpose**: RedoxFS CoW filesystem for VFS `/srv` mount point (see `docs/specs/09b-vfs-native-fs-adr.md`)

### Patch summary (diff from upstream 0.9.0)

`Cargo.toml` changes only — zero source code changes:

1. `libc = "0.2"` → `libc = { version = "0.2", optional = true }` (used only in std/FUSE modules)
2. `redox_syscall = "0.7.5"` → `{ version = "0.7.5", default-features = false }` (needed for `Disk` trait error types; bare-metal compatible)
3. All other deps already had `default-features = false`; added it to `bitflags`, `base64ct`, `endian-num`
4. `features.default = []` (was `["std", "log", "fuse"]`); `std` feature now gates `libc` + `redox_syscall/std`

### Building (no_std)

```sh
cargo check --no-default-features --target riscv64gc-unknown-none-elf -Z build-std=core,alloc
```

### Building (std, for mkfs tooling)

```sh
cargo build --features std --bin redoxfs-mkfs --release
```

### Creating a `/srv` disk image for testing

```sh
# 64 MB blank image
dd if=/dev/zero of=srv.img bs=1M count=64
# Format with RedoxFS
./target/release/redoxfs-mkfs srv.img
# Seed with test data via FUSE (Linux)
mkdir /tmp/srv-mnt
./target/release/redoxfs srv.img /tmp/srv-mnt
echo "ViCell RedoxFS" > /tmp/srv-mnt/hello.txt
fusermount -u /tmp/srv-mnt
```
