#!/bin/bash
set -e
cd /tmp
TOOLS=/tmp/tools-local

# Download all xorriso dependencies for Ubuntu 24.04 Noble
for pkg in libisoburn1t64 libisofs6t64 libburn4t64; do
    apt-get download "$pkg" 2>&1 | grep -E '(Get:|Fetched|Err:)' || true
done

# Extract all to tools-local
for f in /tmp/libisoburn*.deb /tmp/libisofs*.deb /tmp/libburn*.deb; do
    [ -f "$f" ] && echo "Extracting: $(basename $f)" && dpkg-deb -x "$f" "$TOOLS"
done

# List extracted libs
find "$TOOLS/usr/lib" -name "*.so*" 2>/dev/null | head -10

# Verify xorriso works with the local libs
LIB_PATHS="$TOOLS/usr/lib/x86_64-linux-gnu:$TOOLS/usr/lib:$TOOLS/lib/x86_64-linux-gnu"
export LD_LIBRARY_PATH="$LIB_PATHS"
ldd "$TOOLS/usr/bin/xorriso" 2>&1 | grep "not found" || echo "All libs resolved"
"$TOOLS/usr/bin/xorriso" --version 2>&1 | head -2 && echo "XORRISO_READY"
