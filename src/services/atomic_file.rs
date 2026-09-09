use anyhow::{bail, Context};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let (temporary_path, mut temporary_file) = create_temporary_file(path)?;
    let write_result = (|| {
        temporary_file
            .write_all(bytes)
            .with_context(|| format!("failed to write {}", temporary_path.to_string_lossy()))?;
        temporary_file
            .sync_all()
            .with_context(|| format!("failed to flush {}", temporary_path.to_string_lossy()))?;
        Ok::<_, anyhow::Error>(())
    })();
    drop(temporary_file);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }

    if let Err(error) = replace_file(&temporary_path, path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

fn create_temporary_file(path: &Path) -> anyhow::Result<(PathBuf, std::fs::File)> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for attempt in 0..100_u32 {
        let temporary_path = parent.join(format!(
            ".runscope-write-{}-{attempt}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create {}", temporary_path.to_string_lossy())
                });
            }
        }
    }
    bail!(
        "failed to create a unique temporary file next to {}",
        path.to_string_lossy()
    )
}

#[cfg(windows)]
fn replace_file(temporary_path: &Path, path: &Path) -> anyhow::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary_wide = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let target_wide = path
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(temporary_wide.as_ptr()),
            PCWSTR(target_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .with_context(|| format!("failed to atomically replace {}", path.to_string_lossy()))
}

#[cfg(not(windows))]
fn replace_file(temporary_path: &Path, path: &Path) -> anyhow::Result<()> {
    std::fs::rename(temporary_path, path)
        .with_context(|| format!("failed to atomically replace {}", path.to_string_lossy()))
}
