# Changelog

## 2.0.0 — 2026-09-10

### Added
- Opt-in flight recorder, separate from the existing manual process inspector.
- Record AI workload candidates and descendants, a creation-time-verified process tree,
  or all processes. Continue tracking known children after the original root disappears.
- Per-process working set, known VRAM, CPU (100% = one logical processor), cumulative
  and interval process I/O; machine available RAM and swap usage.
- Full-precision process identities prevent PID reuse from silently joining histories.
- First-seen, no-longer-observed, large RAM increase and system-memory-pressure observations.
  Disappearance events preserve the last sample and do not claim to diagnose crashes.
- Bounded, append-only JSONL journals with configurable disk limit; standalone JSON and
  script-free HTML summaries with timelines and last observations.
- Recovery of complete records after a truncated final JSONL write.
- Headless `--record` / `--report`, normal-exit load fixture, native Windows smoke tests.

### Changed
- Version 2.0.0; startup remains idle/manual, recording defaults to OFF.
- Process collector retains sysinfo state for CPU baselines and avoids repeated immutable
  metadata reads; process, GPU and listener work can overlap.
- Replace `typeperf` subprocess with a persistent, language-neutral native Windows PDH
  query behind a bounded, time-limited worker. NVML and nvidia-smi fallbacks remain.
- External helper output is file-backed and bounded, avoiding pipe-buffer deadlocks and
  inherited-grandchild-pipe joins after timeout. Console windows are suppressed.
- Reuse atomic file replacement for settings and derived reports.
- Update the locked webbrowser dependency to 1.2.2 (RUSTSEC-2026-0257, Unix-specific issue).
- Forward launcher arguments and propagate the actual executable exit status.

### Limits / compatibility
- Existing v1 settings, manual reload, filters, local links and guarded process actions remain.
- New journal schema is version 1, independent of the application version.
- HTML/JSON previews retain at most 600 aggregate samples and 256 events; JSONL retains
  every complete recorded sample up to the configured disk limit (default 256 MiB).
- No GPU utilization, exit-code inference, automatic process killing, remote server or telemetry.
- Windows CI does not substitute for GPU-driver, interactive GUI or long-duration hardware testing.
