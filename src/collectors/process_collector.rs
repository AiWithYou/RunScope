use crate::collectors::{gpu_nvidia_smi, gpu_nvml, gpu_windows_perf, local_listeners};
use crate::model::{GpuProcessInfo, ProcessInfo, ProcessSnapshot};
use crate::services::scope_detector;
use crate::settings::Settings;
use anyhow::bail;
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, System, UpdateKind};

pub fn collect_processes(settings: &Settings) -> anyhow::Result<ProcessSnapshot> {
    let (vram_by_pid, vram_status) = collect_vram();
    let (listeners_by_pid, listener_status) = collect_listeners();
    let processes = collect_process_infos(settings, &vram_by_pid, &listeners_by_pid);

    Ok(ProcessSnapshot {
        processes,
        vram_status,
        listener_status,
    })
}

pub fn collect_processes_for_action(settings: &Settings) -> anyhow::Result<Vec<ProcessInfo>> {
    let (listeners_by_pid, _) = collect_listeners();
    Ok(collect_process_infos(
        settings,
        &HashMap::new(),
        &listeners_by_pid,
    ))
}

fn collect_process_infos(
    settings: &Settings,
    vram_by_pid: &HashMap<u32, gpu_nvml::VramUsage>,
    listeners_by_pid: &HashMap<u32, Vec<crate::model::ListeningEndpoint>>,
) -> Vec<ProcessInfo> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessRefreshKind::new()
            .with_memory()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .with_cwd(UpdateKind::Always),
    );

    let mut processes = Vec::with_capacity(system.processes().len());
    let mut names_by_pid = HashMap::new();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        let parent_pid = process.parent().map(|parent| parent.as_u32());
        let command_line = non_empty(process.cmd().join(" "));
        let exe_path = process.exe().map(|path| path.to_string_lossy().to_string());
        let cwd = process.cwd().map(|path| path.to_string_lossy().to_string());
        let start_time = if process.start_time() > 0 {
            Some(UNIX_EPOCH + Duration::from_secs(process.start_time()))
        } else {
            None
        };

        names_by_pid.insert(pid_u32, process.name().to_string());

        let gpu = vram_by_pid.get(&pid_u32).map(|usage| GpuProcessInfo {
            device_index: 0,
            device_name: usage.device_name.clone(),
            vram_bytes: Some(usage.bytes),
            process_type: usage.process_type,
        });

        processes.push(ProcessInfo {
            pid: pid_u32,
            parent_pid,
            parent_name: None,
            name: process.name().to_string(),
            exe_path,
            command_line,
            cwd,
            start_time,
            ram_bytes: process.memory(),
            virtual_memory_bytes: process.virtual_memory(),
            gpu,
            local_endpoints: listeners_by_pid.get(&pid_u32).cloned().unwrap_or_default(),
            children: Vec::new(),
            scope: Default::default(),
            protected: false,
            protection_reason: None,
            python_related: false,
            codex_related: false,
            searchable_text_lower: String::new(),
        });
    }

    let start_times_by_pid = processes
        .iter()
        .map(|process| (process.pid, process.start_time))
        .collect::<HashMap<_, _>>();
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in &processes {
        let Some(parent_pid) = process.parent_pid else {
            continue;
        };
        let parent_start_time = start_times_by_pid.get(&parent_pid).copied().flatten();
        if verified_parent_relation(parent_start_time, process.start_time) {
            children_by_parent
                .entry(parent_pid)
                .or_default()
                .push(process.pid);
        }
    }

    for process in &mut processes {
        process.parent_name = process
            .parent_pid
            .and_then(|pid| names_by_pid.get(&pid).cloned());
        process.children = children_by_parent.remove(&process.pid).unwrap_or_default();
        process.children.sort_unstable();
    }

    scope_detector::assign_scopes(&mut processes, settings);
    for process in &mut processes {
        process.refresh_searchable_text();
    }

    processes
}

fn collect_listeners() -> (HashMap<u32, Vec<crate::model::ListeningEndpoint>>, String) {
    match local_listeners::collect_local_listeners() {
        Ok(map) => {
            let count = map.values().map(Vec::len).sum::<usize>();
            (map, format!("Local listeners: {count}"))
        }
        Err(error) => (
            HashMap::new(),
            format!("Local listeners unavailable: {error}"),
        ),
    }
}

fn collect_vram() -> (HashMap<u32, gpu_nvml::VramUsage>, String) {
    let nvml_result = collect_nvml_with_timeout(Duration::from_millis(1000));
    if let Ok(map) = &nvml_result {
        if !map.is_empty() {
            return (map.clone(), "VRAM: NVML".to_string());
        }
    }

    let windows_perf_result = gpu_windows_perf::collect_vram_by_pid_windows_perf();
    if let Ok(map) = &windows_perf_result {
        return (
            map.clone(),
            match &nvml_result {
                Ok(_) => "VRAM: Windows GPU Process Memory (NVML returned no per-process memory)"
                    .to_string(),
                Err(error) => {
                    format!("VRAM: Windows GPU Process Memory (NVML unavailable: {error})")
                }
            },
        );
    }

    let smi_result = gpu_nvidia_smi::collect_vram_by_pid();
    if let Ok(map) = &smi_result {
        if !map.is_empty() {
            return (
                map.clone(),
                match &nvml_result {
                    Ok(_) => "VRAM: nvidia-smi (NVML returned no per-process memory)".to_string(),
                    Err(error) => format!("VRAM: nvidia-smi (NVML unavailable: {error})"),
                },
            );
        }
    }

    let nvml_status = match nvml_result {
        Ok(_) => "NVML returned no per-process memory".to_string(),
        Err(error) => format!("NVML: {error}"),
    };
    let windows_status = match windows_perf_result {
        Ok(_) => "Windows GPU Process Memory returned no rows".to_string(),
        Err(error) => format!("Windows GPU Process Memory: {error}"),
    };
    let smi_status = match smi_result {
        Ok(_) => "nvidia-smi returned no per-process memory".to_string(),
        Err(error) => format!("nvidia-smi: {error}"),
    };

    (
        HashMap::new(),
        format!("VRAM unavailable: {nvml_status}; {windows_status}; {smi_status}"),
    )
}

pub fn collect_nvml_with_timeout(
    timeout: Duration,
) -> anyhow::Result<HashMap<u32, gpu_nvml::VramUsage>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(gpu_nvml::collect_vram_by_pid_nvml());
    });

    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            bail!("NVML timed out after {}ms", timeout.as_millis());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("NVML worker disconnected");
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn verified_parent_relation(
    parent_start_time: Option<std::time::SystemTime>,
    child_start_time: Option<std::time::SystemTime>,
) -> bool {
    matches!(
        (parent_start_time, child_start_time),
        (Some(parent), Some(child)) if parent <= child
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_current_process_snapshot() {
        let snapshot = collect_processes(&Settings::default()).expect("collect process snapshot");
        let current_pid = std::process::id();
        let current = snapshot
            .processes
            .iter()
            .find(|process| process.pid == current_pid)
            .unwrap_or_else(|| {
                panic!("current PID {current_pid} was not present in the process snapshot")
            });
        assert!(current.protected);
        assert_eq!(
            current.protection_reason.as_deref(),
            Some("Current RunScope process")
        );
        assert!(current
            .searchable_text_lower
            .contains(&current.pid.to_string()));
    }

    #[test]
    fn rejects_parent_relation_when_parent_is_newer_than_child() {
        let parent = UNIX_EPOCH + Duration::from_secs(200);
        let child = UNIX_EPOCH + Duration::from_secs(100);

        assert!(!verified_parent_relation(Some(parent), Some(child)));
    }

    #[test]
    fn rejects_parent_relation_without_both_start_times() {
        let time = UNIX_EPOCH + Duration::from_secs(100);

        assert!(!verified_parent_relation(None, Some(time)));
        assert!(!verified_parent_relation(Some(time), None));
    }

    #[test]
    fn accepts_parent_relation_when_parent_is_not_newer() {
        let parent = UNIX_EPOCH + Duration::from_secs(100);
        let child = UNIX_EPOCH + Duration::from_secs(101);

        assert!(verified_parent_relation(Some(parent), Some(child)));
        assert!(verified_parent_relation(Some(parent), Some(parent)));
    }
}
