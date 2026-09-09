"""Windows native smoke test of the built executable and recovery path.

Uses only its own bounded fixture processes, never terminates unrelated PIDs.
No machine-specific logs are printed or uploaded; temporary recordings are removed.
"""
from __future__ import annotations
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def checked(command: list[str], timeout: int = 90) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {command[0]}\n{result.stderr}")
    return result


def main() -> int:
    exe = Path(sys.argv[1] if len(sys.argv) > 1 else "dist/RunScope.exe").resolve()
    assert checked([str(exe), "--version"]).stdout.strip() == "RunScope 2.0.0"
    assert subprocess.run([str(exe), "--record", "unused", "--interval", "0"], capture_output=True).returncode == 2
    with tempfile.TemporaryDirectory(prefix="runscope-smoke-") as folder:
        fixture = subprocess.Popen([sys.executable, str(Path(__file__).with_name("diagnostic_workload.py")), "--seconds", "10"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            # Allow the fixture to create its child before the first collection.
            time.sleep(1)
            checked([str(exe), "--record", folder, "--pid", str(fixture.pid), "--duration", "16", "--interval", "1"])
            fixture.wait(timeout=10)
            assert fixture.returncode == 0
        finally:
            if fixture.poll() is None:
                # Finish naturally where possible; kill only our own fixture on failure.
                try:
                    fixture.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    fixture.kill()
                    fixture.wait()
            if fixture.stderr:
                fixture.stderr.close()
        sessions = list(Path(folder).glob("session-*"))
        assert len(sessions) == 1
        session = sessions[0]
        log = session / "samples.jsonl"
        records = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]
        frames = [r["data"] for r in records if r["record"] == "frame"]
        assert len(frames) >= 3, "insufficient observations"
        assert records[0]["record"] == "header" and records[-1]["record"] == "end"
        all_processes = [p for frame in frames for p in frame["processes"]]
        assert any(p["pid"] == fixture.pid for p in all_processes)
        assert any(p["parent_pid"] == fixture.pid for p in all_processes), "child was not tracked"
        assert all("sensitive" not in p for p in all_processes)
        assert any(p["cpu_percent"] is not None for p in all_processes), "no CPU baseline"
        assert any((p["io_write_bytes_delta"] or 0) > 0 for p in all_processes), "no I/O growth"
        assert any(e["kind"] == "no_longer_observed" for frame in frames for e in frame["events"]), "exit not observed"
        report = json.loads((session / "report.json").read_text(encoding="utf-8"))
        assert report["summary"]["sample_count"] == len(frames)
        assert (session / "report.html").read_text(encoding="utf-8").startswith("<!doctype html>")
        # Reproduce an interrupted write, without mutating the original journal.
        interrupted = session / "interrupted.jsonl"
        prefix = [r for r in records if r["record"] != "end"]
        interrupted.write_text("".join(json.dumps(r) + "\n" for r in prefix) + '{"record":', encoding="utf-8")
        checked([str(exe), "--report", str(interrupted)])
        recovered = json.loads((session / "report.json").read_text(encoding="utf-8"))
        assert recovered["summary"]["sample_count"] == len(frames)
        assert "incomplete" in recovered["completion"]
    print("PASS: version, argument validation, native process tree, CPU, I/O, exit observations, exports, interrupted-log recovery")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
