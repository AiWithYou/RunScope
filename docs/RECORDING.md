# Flight recording (v2)

## Reproduce a slowdown or disappearance

1. Open RunScope. For a specific process, click **Load / Reload**, then select the row.
2. Open **Flight recorder**, choose **Selected process tree**, and start before reproduction.
   **AI workloads + children** uses name/path/command-line heuristics; **All processes** is broader.
3. Reproduce the issue. Close the recorder window to keep working; recording continues.
4. Stop with **Stop and write reports**. Open the HTML report or the session directory.

Recording is opt-in and independent of table filters. Inspector Auto refresh is a separate
collector; leave it OFF when only the recorder is needed. Closing RunScope requests a stop
and completes the in-flight sample and report writes. Force-killing it cannot finalize the
report, but the complete JSONL prefix remains recoverable.

The GUI defaults to one sample every 5 seconds and a 256 MiB log cap. Shorter intervals add
collector overhead and are not guaranteed on every machine. Collection overruns are counted;
there is no rapid catch-up loop. Minimized windows do not stop the recording worker.

## CLI (PowerShell)

```powershell
# One minute, AI workload candidates and their descendants; no GUI.
.\RunScope.exe --record "$env:LOCALAPPDATA\RunScope\recordings"

# Track PID 12345 and its verified descendants for five minutes.
.\RunScope.exe --record .\recordings --pid 12345 --duration 300 --interval 2

# Explicitly record all processes; cap the journal at 64 MiB.
.\RunScope.exe --record .\recordings --all --duration 60 --max-log-mib 64

# Rebuild summaries after an interrupted write. Do not run against a log still being written.
.\RunScope.exe --report .\recordings\session-...\samples.jsonl
```

CLI duration defaults to 60 seconds; allowed range 1..604800. Interval range is 1..60 seconds,
log size 1..4096 MiB. `--pid` and `--all` are mutually exclusive. Invalid arguments exit 2;
collection/export errors exit 1; planned stop or limit completion exits 0. CTRL+C/forced
termination of the headless recorder may require `--report` recovery.

## Files and schema

Each start creates a new `session-<Unix-ms>-<recorder-PID>-<suffix>` subdirectory, never reuses
an existing recording, and writes:

| File | Contents |
| --- | --- |
| `samples.jsonl` | UTF-8, one `header`, consecutive `frame` records, optional final `end` record. Full recorded history. |
| `report.json` | Header, completion status, all-time aggregate peaks, bounded timeline/events and last process observations. |
| `report.html` | Offline, escaped, script-free report. No external assets or network requests. |

Each JSONL record has `record` and `data` keys. Header schema version is **1**. Frames carry
sequence, monotonic elapsed milliseconds, wall-clock observation time, collector duration,
processes, known-value aggregates, collector status and events. A process identity is a PID
plus **decimal-string nanoseconds since Unix epoch**, preserving Windows creation-time precision
without JavaScript integer rounding. Unverifiable identities stay null; destructive actions do
not use recorder data. Tree mode requires verified creation times.

Unknown VRAM/CPU/I/O values are `null`, not zero. First CPU/I/O deltas require a baseline;
I/O counter resets do not underflow. RAM is a process working set. Summing working sets can
double-count shared physical pages. CPU 100% means one logical CPU, not 100% of the whole machine.
Process I/O includes operations that may not correspond to physical disk traffic. Known VRAM
sums can be incomplete; a decrease in collector coverage is not proof of memory release.
No GPU utilization is measured. Workload labels are heuristics, not verified identities.

`first_seen` means first observed in scope, not necessarily newly created. `no_longer_observed`
means absent from the next scope: normal exit, denied access or a collection gap are all possible.
The last observed process values are retained in the event. `ram_growth` is a >=256 MiB increase
between two observations; `system_memory_pressure` marks available system memory below 10%.
These are investigation clues, not crash, memory leak or out-of-memory verdicts.

## Bounds, recovery and privacy

Only 600 aggregate preview points and 256 recent events are kept in summaries. Omission counters
make that explicit. The GUI retains a 120-point preview. Full JSONL records remain available until
the configured disk cap; recording stops rather than silently rotating/deleting history. A single
record is limited to 4 MiB. A forced termination or storage error may leave one incomplete tail
record; recovery discards only that incomplete final record and rejects corruption in the middle.
Report recovery atomically replaces derived JSON/HTML files, never the source JSONL.

Every complete sample is written/flushed to the OS; `sync_all` is used at graceful completion.
This is not a guarantee against OS crashes, power loss, or failing storage. A file-size cap does
not reserve free disk space. Errors are reported, and the completed prefix can be recovered.

Command lines, executable paths and CWD are **excluded by default**. Use **Include ...** in the GUI
or `--include-sensitive` only when needed; these values can contain API tokens and private paths.
They are capped at 4096 characters per field. Defaults still include names, PIDs, creation times,
port numbers and metrics, so exports are not anonymous. Review all files before sharing them.
No auto-upload or network listener is added.

## Reproducible check

```powershell
python .\scripts\diagnostic_workload.py --seconds 20
# Select its printed root PID and record it, including several seconds after it exits.
python .\scripts\smoke_recording.py .\dist\RunScope.exe
```

The fixture allocates at most 32 MiB of retained blocks, writes a bounded temporary file,
spawns one child and exits normally. It is not a GPU benchmark. The smoke test checks identity,
child tracking, CPU/I/O observations, disappearance events, export and tail recovery. CI removes
its machine-specific recordings and prints only verification results.

For a GPU hardware check, record a real ComfyUI/Forge workload through loading, generation and
unloading. Compare known VRAM and the reported collector source with the relevant GPU diagnostic
tool. NVIDIA/WDDM availability varies; N/A remains valid. Do not equate reported per-process
sums with total device VRAM or compare numbers collected at different times as exact matches.
