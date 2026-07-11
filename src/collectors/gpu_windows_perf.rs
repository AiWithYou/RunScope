use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context};

use crate::collectors::gpu_nvml::VramUsage;
use crate::model::GpuProcessType;
use crate::services::command_timeout;

pub fn collect_vram_by_pid_windows_perf() -> anyhow::Result<HashMap<u32, VramUsage>> {
    let mut command = Command::new("typeperf");
    command.args([r"\GPU Process Memory(*)\Dedicated Usage", "-sc", "1"]);
    let output =
        command_timeout::output_with_timeout(&mut command, "typeperf", Duration::from_millis(2500))
            .context("failed to run typeperf")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("typeperf exited with {}: {}", output.status, stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let csv_text = stdout
        .lines()
        .filter(|line| line.starts_with('"'))
        .collect::<Vec<_>>()
        .join("\n");

    if csv_text.trim().is_empty() {
        bail!("typeperf returned no GPU Process Memory rows");
    }

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_text.as_bytes());
    let mut records = reader.records();
    let Some(header) = records.next() else {
        bail!("typeperf returned no CSV header");
    };
    let header = header?;
    let Some(values) = records.next() else {
        bail!("typeperf returned no CSV values");
    };
    let values = values?;

    let mut map = HashMap::new();
    for index in 1..header.len().min(values.len()) {
        let Some(pid) = pid_from_counter_path(&header[index]) else {
            continue;
        };
        let Ok(bytes_float) = values[index].parse::<f64>() else {
            continue;
        };
        if !bytes_float.is_finite() || bytes_float <= 0.0 {
            continue;
        }
        let bytes = bytes_float.round() as u64;
        map.entry(pid)
            .and_modify(|usage: &mut VramUsage| {
                usage.bytes = usage.bytes.saturating_add(bytes);
            })
            .or_insert_with(|| VramUsage {
                bytes,
                device_indices: Vec::new(),
                device_names: vec!["Windows GPU Process Memory".to_string()],
                process_type: GpuProcessType::Unknown,
            });
    }

    if map.is_empty() {
        bail!("typeperf returned no non-zero dedicated GPU memory rows");
    }

    Ok(map)
}

fn pid_from_counter_path(path: &str) -> Option<u32> {
    let marker = "GPU Process Memory(pid_";
    let start = path.find(marker)? + marker.len();
    let rest = &path[start..];
    let end = rest.find('_')?;
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::pid_from_counter_path;

    #[test]
    fn parses_typeperf_pid() {
        let path = r"\\HOST\GPU Process Memory(pid_34728_luid_0x00000000_0x0001179E_phys_0)\Dedicated Usage";
        assert_eq!(pid_from_counter_path(path), Some(34728));
    }
}
