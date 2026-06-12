#!/usr/bin/env python3
"""Boot-test ViCell AArch64 on QEMU, assert shell prompt reached."""

import subprocess, socket, time, sys

KERNEL = "target/aarch64-unknown-none-softfloat/release/vicell-kernel"
DISK   = "disk_arm_virt.img"
QEMU   = r"C:\Program Files\qemu\qemu-system-aarch64.exe"
PORT   = 55204

# Listen first so QEMU connects to us — no bytes lost from early boot output.
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", PORT))
srv.listen(1)
print(f"[test] Listening on tcp:127.0.0.1:{PORT} (QEMU will connect as client)...")

args = [
    QEMU,
    "-machine", "virt", "-cpu", "cortex-a57", "-m", "256M", "-nographic",
    "-kernel", KERNEL,
    "-drive", f"if=none,file={DISK},format=raw,id=hd0",
    "-device", "virtio-blk-device,drive=hd0",
    "-netdev", "user,id=net0", "-device", "virtio-net-device,netdev=net0",
    "-no-reboot", "-monitor", "none",
    "-serial", f"tcp:127.0.0.1:{PORT}",  # QEMU connects to us (no server flag)
]

print("[test] Spawning QEMU...")
proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

srv.settimeout(8.0)
try:
    conn, _ = srv.accept()
    print("[test] QEMU connected to serial socket.")
except Exception as e:
    print(f"FAIL: QEMU did not connect to serial socket: {e}")
    proc.kill()
    sys.exit(1)
finally:
    srv.close()

print("[test] Reading serial output (45s timeout)...")
output = b""
deadline = time.time() + 45
while time.time() < deadline:
    try:
        chunk = conn.recv(4096)
        if chunk:
            output += chunk
            if b"ViCell >" in output:
                break
    except socket.timeout:
        pass
    except Exception:
        break

conn.close()
proc.kill()
proc.wait()

text = output.decode("utf-8", errors="replace")
lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")

print("=== SERIAL OUTPUT (last 80 lines) ===")
for l in lines[-80:]:
    print(l)
print()

checks = [
    ("[ViCell] kernel boot", "kernel banner"),
    ("Scheduler initialized",  "scheduler init"),
    ("Successfully spawned init", "init spawned"),
    ("ViCell >",               "shell prompt"),
]

all_pass = True
for marker, desc in checks:
    ok = marker in text
    sym = "OK" if ok else "FAIL"
    print(f"  [{sym}]  {desc:30s}  ('{marker}')")
    if not ok:
        all_pass = False

print()
if all_pass:
    print("RESULT: PASS — all markers found")
    sys.exit(0)
else:
    print("RESULT: FAIL — some markers missing")
    sys.exit(1)
