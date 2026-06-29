use std::process::{Command, Output, Stdio};
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
    let start = Instant::now();

    loop {
        if child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
            .is_some()
        {
            return child
                .wait_with_output()
                .with_context(|| format!("failed to collect {label} output"));
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{label} timed out after {}ms", timeout.as_millis());
        }

        thread::sleep(Duration::from_millis(20));
    }
}
