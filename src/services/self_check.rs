use std::time::Duration;

use anyhow::{bail, Context};

use crate::collectors::{gpu_nvidia_smi, gpu_windows_perf, local_listeners, process_collector};
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
    lines.push(if settings_path.exists() {
        format!("settings: loaded ({})", settings_path.to_string_lossy())
    } else {
        format!(
            "settings: built-in defaults (no file at {})",
            settings_path.to_string_lossy()
        )
    });

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
        Ok(map) => lines.push(format!("nvml query: ok ({} active processes)", map.len())),
        Err(error) => lines.push(format!("nvml: unavailable ({error})")),
    }

    match gpu_windows_perf::collect_vram_by_pid_windows_perf() {
        Ok(map) => lines.push(format!(
            "Windows GPU Process Memory query: ok ({} active processes)",
            map.len()
        )),
        Err(error) => lines.push(format!("Windows GPU Process Memory: unavailable ({error})")),
    }

    match gpu_nvidia_smi::collect_vram_by_pid() {
        Ok(map) => lines.push(format!(
            "nvidia-smi query: ok ({} active processes)",
            map.len()
        )),
        Err(error) => lines.push(format!("nvidia-smi query: unavailable ({error})")),
    }

    Ok(lines.join("\n"))
}
