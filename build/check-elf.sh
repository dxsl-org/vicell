#!/bin/bash
ELF=/mnt/d/ViCell/target/x86_64-unknown-none/release/vicell-kernel
echo "=== file ==="
file "$ELF"
echo "=== readelf header ==="
readelf -h "$ELF" | grep -E '(Type|Machine|Entry|Class)'
echo "=== sections ==="
readelf -S "$ELF" | grep -E '(requests|stack|Name)' | head -20
echo "=== ELF size ==="
ls -lh "$ELF"
