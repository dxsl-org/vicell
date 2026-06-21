#!/usr/bin/env bash
# Creates a ViCell disk image for CI benchmarking (direct QEMU boot, no Limine).
#
# The disk contains only the ViCell cell bootstrap table at CELL_TABLE_BASE_LBA.
# Boot: qemu-system-riscv64 -kernel <vicell-kernel> -drive file=bench-disk.img,...
# init reads the cell table → auto-spawns /bin/bench.
#
# Usage:
#   ./scripts/gen-bench-disk.sh <bench-elf> [output-disk]
#
# Arguments:
#   bench-elf    path to compiled app-bench ELF
#   output-disk  output disk image path (default: bench-disk.img)

set -euo pipefail

BENCH_BIN="${1:?Usage: $0 <bench-elf> [output-disk]}"
DISK="${2:-bench-disk.img}"

[[ -f "$BENCH_BIN" ]] || { echo "[gen-bench-disk] ERROR: not found: $BENCH_BIN" >&2; exit 1; }

echo "[gen-bench-disk] Writing cell bootstrap table (/bin/bench)..."
touch "$DISK"
python3 tools/write-cell-table.py "$DISK" "/bin/bench=$BENCH_BIN" "/bin/bench-probe=${BENCH_BIN}-probe"

DISK_MB=$(( $(stat -c%s "$DISK") / 1024 / 1024 ))
echo "[gen-bench-disk] Done: $DISK (${DISK_MB} MB)"
echo "[gen-bench-disk]   Cell table: /bin/bench at CELL_TABLE_BASE_LBA (for EarlyLoader)"
