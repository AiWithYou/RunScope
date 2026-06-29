use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context};

use crate::collectors::{local_listeners, process_collector};
use crate::services::command_timeout;
use crate::settings::Settings;

pub fn run() -> anyhow::Result<String> {
    let settings_path = Settings::default_path();
    let settings = Settings::load_or_default(&settings_path).with_context(|| {
        format!(
            "failed to load settings from {}",
            settings_path.to_string_lossy()
        )
    })?;

    let mut lines = Vec::new();
    lines.push(format!("RunScope {}", env!("CARGO_PKG_VERSION")));
    lines.push(format!(
        "settings: ok ({})",
        settings_path.to_string_lossy()
    ));

    let processes = process_collector::collect_processes_for_action(&settings)?;
    if processes.is_empty() {
        bail!("process snapshot returned no processes");
    }
    lines.push(format!(
        "process snapshot: ok ({} processes)",
        processes.len()
    ));

    match local_listeners::collect_local_listeners() {
        Ok(listeners) => {
            let count = listeners.values().map(Vec::len).sum::<usize>();
            lines.push(format!("local listeners: ok ({count} endpoints)"));
        }
        Err(error) => {
            lines.push(format!("local listeners: unavailable ({error})"));
        }
    }

    match process_collector::collect_nvml_with_timeout(Duration::from_millis(1000)) {
        Ok(map) => lines.push(format!("nvml: ok ({} processes)", map.len())),
        Err(error) => lines.push(format!("nvml: unavailable ({error})")),
    }

    let mut nvidia_smi = Command::new("nvidia-smi");
    nvidia_smi.arg("--version");
    match command_timeout::output_with_timeout(
        &mut nvidia_smi,
        "nvidia-smi --version",
        Duration::from_millis(2500),
    ) {
        Ok(output) if output.status.success() => {
            lines.push("nvidia-smi: ok".to_string());
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            lines.push(format!(
                "nvidia-smi: unavailable (exit {}: {})",
                output.status,
                stderr.trim()
            ));
        }
        Err(error) => {
            lines.push(format!("nvidia-smi: unavailable ({error})"));
        }
    }

    Ok(lines.join("\n"))
}
