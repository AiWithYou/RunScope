use std::io::Read;
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

pub fn output_with_timeout(
    command: &mut Command,
    label: &str,
    timeout: Duration,
) -> anyhow::Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    let stdout = child
        .stdout
        .take()
        .with_context(|| format!("failed to capture {label} stdout"))?;
    let stderr = child
        .stderr
        .take()
        .with_context(|| format!("failed to capture {label} stderr"))?;
    let stdout_reader = thread::spawn(move || read_stdout(stdout));
    let stderr_reader = thread::spawn(move || read_stderr(stderr));
    let start = Instant::now();

    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            return collect_output(status, stdout_reader, stderr_reader, label);
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            bail!("{label} timed out after {}ms", timeout.as_millis());
        }

        thread::sleep(Duration::from_millis(20));
    }
}

fn read_stdout(mut stdout: ChildStdout) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stdout.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_stderr(mut stderr: ChildStderr) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stderr.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn collect_output(
    status: ExitStatus,
    stdout_reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    label: &str,
) -> anyhow::Result<Output> {
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("{label} stdout reader panicked"))?
        .with_context(|| format!("failed to read {label} stdout"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("{label} stderr reader panicked"))?
        .with_context(|| format!("failed to read {label} stderr"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn drains_large_child_output_without_pipe_deadlock() {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "(for /L %i in (1,1,20000) do @echo 1234567890)"]);

        let output =
            output_with_timeout(&mut command, "large-output-test", Duration::from_secs(10))
                .expect("large child output should be drained");

        assert!(output.status.success());
        assert!(output.stdout.len() > 200_000);
    }
}
