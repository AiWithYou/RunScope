"""Small, deterministic process-tree / RAM / I/O fixture; no third-party packages.

Run: python scripts/diagnostic_workload.py --seconds 20
Memory growth is capped at 32 MiB; files live in TemporaryDirectory and are removed.
This is a normal-exit fixture, not a simulated crash or a GPU benchmark.
"""
from __future__ import annotations
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=int, default=20)
    parser.add_argument("--child", action="store_true")
    args = parser.parse_args()
    if not 3 <= args.seconds <= 120:
        parser.error("--seconds must be 3..120")
    if args.child:
        time.sleep(args.seconds)
        return 0
    print(f"Root PID: {os.getpid()}", flush=True)
    child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--child", "--seconds", str(args.seconds)])
    print(f"Child PID: {child.pid}", flush=True)
    blocks: list[bytearray] = []
    try:
        with tempfile.TemporaryDirectory(prefix="runscope-fixture-") as folder:
            started = time.monotonic()
            with (Path(folder) / "io.bin").open("wb", buffering=0) as output:
                while time.monotonic() - started < args.seconds:
                    if len(blocks) < 8:
                        # Touch actual pages, rather than only reserving address space.
                        blocks.append(bytearray(b"R" * (4 * 1024 * 1024)))
                    output.seek(0)
                    output.write(blocks[-1])
                    output.flush()
                    os.fsync(output.fileno())
                    time.sleep(1)
    finally:
        child.wait(timeout=args.seconds + 5)
    print("Fixture exited normally.", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
