#!/bin/bash
set -e
TOOLS=/tmp/tools-local

dpkg-deb -x /tmp/mtools_4.0.43-1build1_amd64.deb $TOOLS
echo "mcopy at: $(find $TOOLS -name 'mcopy' 2>/dev/null)"
$TOOLS/usr/bin/mcopy --version 2>&1 | head -2 || true
echo "MTOOLS_READY"
