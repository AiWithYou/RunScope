use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;

/// File-backed capture cannot deadlock on full pipes or on a grandchild holding
/// an inherited pipe open. Both output size and execution time are bounded.
pub fn output_with_timeout(
    command: &mut Command,
    label: &str,
    timeout: Duration,
) -> anyhow::Result<Output> {
    let mut stdout = Capture::new()?;
    let mut stderr = Capture::new()?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.file.try_clone()?))
        .stderr(Stdio::from(stderr.file.try_clone()?))
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    // Command owns its configured handles too; release them, including on reuse.
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let start = Instant::now();
    let result = (|| loop {
        if stdout.file.metadata()?.len() > MAX_OUTPUT_BYTES
            || stderr.file.metadata()?.len() > MAX_OUTPUT_BYTES
        {
            bail!(
                "{label} output exceeded {} MiB per stream",
                MAX_OUTPUT_BYTES / 1024 / 1024
            );
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            return Ok(Output {
                status,
                stdout: stdout.read()?,
                stderr: stderr.read()?,
            });
        }
        if start.elapsed() >= timeout {
            bail!("{label} timed out after {}ms", timeout.as_millis());
        }
        thread::sleep(Duration::from_millis(20).min(timeout.saturating_sub(start.elapsed())));
    })();
    if result.is_err() {
        // Only terminate the helper we created, never arbitrary observed PIDs.
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

struct Capture {
    file: File,
    path: PathBuf,
}

impl Capture {
    fn new() -> anyhow::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "runscope-capture-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.custom_flags(0x04000000); // FILE_FLAG_DELETE_ON_CLOSE
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok(Self { file, path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("failed to create helper output capture"),
            }
        }
        bail!("failed to create a unique helper output capture")
    }

    fn read(&mut self) -> anyhow::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut self.file)
            .take(MAX_OUTPUT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_OUTPUT_BYTES {
            bail!("helper output exceeded the capture limit");
        }
        Ok(bytes)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn drains_large_child_output_without_pipe_deadlock() {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "(for /L %i in (1,1,20000) do @echo 1234567890)"]);
        let output =
            output_with_timeout(&mut command, "large-output-test", Duration::from_secs(10))
                .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() > 200_000);
    }

    #[test]
    fn timeout_returns_without_joining_inherited_output_readers() {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "ping -n 4 127.0.0.1 >nul"]);
        let start = Instant::now();
        let error = output_with_timeout(&mut command, "timeout-test", Duration::from_millis(100))
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn captures_both_streams_and_preserves_failure_status() {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "echo out & echo err 1>&2 & exit /b 7"]);
        let output = output_with_timeout(&mut command, "streams", Duration::from_secs(5)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert!(String::from_utf8_lossy(&output.stdout).contains("out"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("err"));
    }
}
