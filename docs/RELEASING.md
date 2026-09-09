# Release checks

Normal main pushes run Windows formatting, compilation, unit tests, Clippy,
dependency advisory checks and native recording/recovery smoke tests.

To release a new stable version, update Cargo.toml and Cargo.lock, add its section
headed `## MAJOR.MINOR.PATCH ` to CHANGELOG.md, then include `[release]` in the main
commit message. Publishing runs only after both build and advisory jobs succeed.
It downloads the package from that exact workflow run, verifies SHA256 and
packaged-EXE equality, and targets the exact validated commit. It refuses an existing
tag or a main branch that advanced during validation. Tags and published assets
are never overwritten by this path. Ordinary commits and pull requests do not
publish. A failed upload may leave a draft requiring maintainer review; do not
silently replace a previously published version.

The existing `vMAJOR.MINOR.PATCH` tag workflow remains available and also runs the
native recording smoke test before uploading release assets.

## Local verification

From a source checkout on Windows:

```powershell
cargo fmt --all --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
.\build_release.ps1
python .\scripts\smoke_recording.py .\dist\RunScope.exe
```

From an extracted release ZIP, Python is needed only for the optional fixture test:

```powershell
python .\scripts\smoke_recording.py .\RunScope.exe
```

The Rust application itself needs no Python runtime. Read [RECORDING.md](RECORDING.md)
for field meanings, storage bounds, privacy defaults and the real-GPU verification
procedure. CI's fixture is not a GPU hardware benchmark or an interactive GUI test.
