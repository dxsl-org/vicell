@echo off
cd /d "d:\ViCell"
cargo check --target x86_64-unknown-none -Z build-std=core,alloc > d:\ViCell\__build_out.txt 2>&1
echo EXIT=%ERRORLEVEL% >> d:\ViCell\__build_out.txt
