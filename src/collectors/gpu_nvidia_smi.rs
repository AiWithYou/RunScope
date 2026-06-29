use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context};

use crate::collectors::gpu_nvml::VramUsage;
use crate::model::GpuProcessType;
use crate::services::command_timeout;

pub fn collect_vram_by_pid() -> anyhow::Result<HashMap<u32, VramUsage>> {
    let mut command = Command::new("nvidia-smi");
    command.args([
        "--query-compute-apps=pid,used_gpu_memory",
        "--format=csv,noheader,nounits",
    ]);
    let output = command_timeout::output_with_timeout(
        &mut command,
        "nvidia-smi",
        Duration::from_millis(2500),
    )
    .context("failed to run nvidia-smi")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "nvidia-smi exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();

    for line in stdout.lines() {
        let mut parts = line.split(',').map(str::trim);
        let Some(pid_text) = parts.next() else {
            continue;
        };
        let Some(memory_text) = parts.next() else {
            continue;
        };
        let Ok(pid) = pid_text.parse::<u32>() else {
            continue;
        };
        let Ok(memory_mib) = memory_text.parse::<u64>() else {
            continue;
        };
        if pid == 0 || memory_mib == 0 {
            continue;
        }

        let bytes = memory_mib.saturating_mul(1024 * 1024);
        map.entry(pid)
            .and_modify(|usage: &mut VramUsage| {
                usage.bytes = usage.bytes.saturating_add(bytes);
            })
            .or_insert_with(|| VramUsage {
                bytes,
                device_name: "NVIDIA".to_string(),
                process_type: GpuProcessType::Compute,
            });
    }

    Ok(map)
}
