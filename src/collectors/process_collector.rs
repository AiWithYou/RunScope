use crate::collectors::{gpu_nvidia_smi, gpu_nvml, gpu_windows_perf, local_listeners};
use crate::model::{GpuProcessInfo, ProcessInfo, ProcessSnapshot, SnapshotState};
use crate::services::scope_detector;
use crate::settings::Settings;
use anyhow::bail;
use std::collections::HashMap;
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, System, UpdateKind};

pub fn collect_processes(settings: &Settings) -> anyhow::Result<ProcessSnapshot> {
    let total_started = Instant::now();
    let (
        ((vram_by_pid, vram_status), vram_elapsed),
        ((listeners_by_pid, listener_status), listener_elapsed),
    ) = thread::scope(|scope| {
        let vram_worker = scope.spawn(|| {
            let started = Instant::now();
            (collect_vram(), started.elapsed())
        });
        let listener_worker = scope.spawn(|| {
            let started = Instant::now();
            (collect_listeners(), started.elapsed())
        });
        let vram = vram_worker.join().unwrap_or_else(|_| {
            (
                (
                    HashMap::new(),
                    "VRAM unavailable: collector worker panicked".to_string(),
                ),
                Duration::ZERO,
            )
        });
        let listeners = listener_worker.join().unwrap_or_else(|_| {
            (
                (
                    HashMap::new(),
                    "Local listeners unavailable: collector worker panicked".to_string(),
                ),
                Duration::ZERO,
            )
        });
        (vram, listeners)
    });
    let process_started = Instant::now();
    let processes = collect_process_infos(settings, vram_by_pid, listeners_by_pid);
    let process_elapsed = process_started.elapsed();

    Ok(ProcessSnapshot {
        processes,
        vram_status,
        listener_status,
        timing_status: format!(
            "Load: {} (VRAM {}, listeners {}, processes {})",
            duration_text(total_started.elapsed()),
            duration_text(vram_elapsed),
            duration_text(listener_elapsed),
            duration_text(process_elapsed)
        ),
    })
}

fn duration_text(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.2}s", duration.as_secs_f64())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

pub fn collect_processes_for_action(settings: &Settings) -> anyhow::Result<Vec<ProcessInfo>> {
    Ok(collect_process_infos(
        settings,
        HashMap::new(),
        HashMap::new(),
    ))
}

fn collect_process_infos(
    settings: &Settings,
    mut vram_by_pid: HashMap<u32, gpu_nvml::VramUsage>,
    mut listeners_by_pid: HashMap<u32, Vec<crate::model::ListeningEndpoint>>,
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

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        let parent_pid = process.parent().map(|parent| parent.as_u32());
        let name = process.name().to_string();
        let command_line = non_empty(process.cmd().join(" "));
        let exe_path = process.exe().map(|path| path.to_string_lossy().to_string());
        let cwd = process.cwd().map(|path| path.to_string_lossy().to_string());
        let start_time = if process.start_time() > 0 {
            Some(UNIX_EPOCH + Duration::from_secs(process.start_time()))
        } else {
            None
        };

        let gpu = vram_by_pid.remove(&pid_u32).map(|usage| GpuProcessInfo {
            device_indices: usage.device_indices,
            device_names: usage.device_names,
            vram_bytes: Some(usage.bytes),
            process_type: usage.process_type,
        });

        processes.push(ProcessInfo {
            pid: pid_u32,
            parent_pid,
            parent_name: None,
            name,
            exe_path,
            command_line,
            cwd,
            start_time,
            ram_bytes: process.memory(),
            virtual_memory_bytes: process.virtual_memory(),
            ram_delta_bytes: None,
            vram_delta_bytes: None,
            snapshot_state: SnapshotState::Unavailable,
            gpu,
            local_endpoints: listeners_by_pid.remove(&pid_u32).unwrap_or_default(),
            children: Vec::new(),
            scope: Default::default(),
            protected: false,
            protection_reason: None,
            python_related: false,
            codex_related: false,
            searchable_text_lower: String::new(),
        });
    }

    retain_verified_parent_relations(&mut processes);

    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in &processes {
        if let Some(parent_pid) = process.parent_pid {
            children_by_parent
                .entry(parent_pid)
                .or_default()
                .push(process.pid);
        }
    }

    let index_by_pid = processes
        .iter()
        .enumerate()
        .map(|(index, process)| (process.pid, index))
        .collect::<HashMap<_, _>>();
    for index in 0..processes.len() {
        let parent_name = processes[index]
            .parent_pid
            .and_then(|pid| index_by_pid.get(&pid))
            .map(|parent_index| processes[*parent_index].name.clone());
        let mut children = children_by_parent
            .remove(&processes[index].pid)
            .unwrap_or_default();
        children.sort_unstable();
        processes[index].parent_name = parent_name;
        processes[index].children = children;
    }

    scope_detector::assign_scopes(&mut processes, settings);
    // The UI builds the search index after applying snapshot delta state. Action snapshots do not
    // use the index, so building it here would duplicate work on every visible reload.

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
    let nvml_result = match collect_nvml_with_timeout(Duration::from_millis(1000)) {
        Ok(map) if !map.is_empty() => return (map, "VRAM: NVML".to_string()),
        result => result,
    };

    let windows_perf_result = match gpu_windows_perf::collect_vram_by_pid_windows_perf() {
        Ok(map) if !map.is_empty() => {
            let status = match &nvml_result {
                Ok(_) => "VRAM: Windows GPU Process Memory (NVML returned no per-process memory)"
                    .to_string(),
                Err(error) => {
                    format!("VRAM: Windows GPU Process Memory (NVML unavailable: {error})")
                }
            };
            return (map, status);
        }
        result => result,
    };

    let smi_result = match gpu_nvidia_smi::collect_vram_by_pid() {
        Ok(map) if !map.is_empty() => {
            let status = match &nvml_result {
                Ok(_) => "VRAM: nvidia-smi (NVML returned no per-process memory)".to_string(),
                Err(error) => format!("VRAM: nvidia-smi (NVML unavailable: {error})"),
            };
            return (map, status);
        }
        result => result,
    };

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
    type NvmlResult = anyhow::Result<HashMap<u32, gpu_nvml::VramUsage>>;
    type NvmlRequest = mpsc::Sender<NvmlResult>;
    static NVML_WORKER: OnceLock<Result<mpsc::SyncSender<NvmlRequest>, String>> = OnceLock::new();

    let worker = NVML_WORKER.get_or_init(|| {
        let (request_sender, request_receiver) = mpsc::sync_channel::<NvmlRequest>(1);
        thread::Builder::new()
            .name("runscope-nvml".to_string())
            .spawn(move || {
                while let Ok(response_sender) = request_receiver.recv() {
                    let _ = response_sender.send(gpu_nvml::collect_vram_by_pid_nvml());
                }
            })
            .map(|_| request_sender)
            .map_err(|error| error.to_string())
    });
    let worker = worker
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to start NVML worker: {error}"))?;
    let (sender, receiver) = mpsc::channel();
    match worker.try_send(sender) {
        Ok(()) => {}
        Err(mpsc::TrySendError::Full(_)) => {
            bail!("NVML worker is still busy from an earlier request");
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            bail!("NVML worker disconnected");
        }
    }

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

fn retain_verified_parent_relations(processes: &mut [ProcessInfo]) {
    let start_times_by_pid = processes
        .iter()
        .map(|process| (process.pid, process.start_time))
        .collect::<HashMap<_, _>>();
    for process in processes {
        let Some(parent_pid) = process.parent_pid else {
            continue;
        };
        let parent_start_time = start_times_by_pid.get(&parent_pid).copied().flatten();
        if parent_pid == process.pid
            || !verified_parent_relation(parent_start_time, process.start_time)
        {
            process.parent_pid = None;
        }
    }
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

    #[test]
    fn removes_unverified_and_self_referencing_parent_metadata() {
        let parent_start = UNIX_EPOCH + Duration::from_secs(200);
        let child_start = UNIX_EPOCH + Duration::from_secs(100);
        let mut processes = vec![
            ProcessInfo {
                pid: 10,
                start_time: Some(parent_start),
                ..Default::default()
            },
            ProcessInfo {
                pid: 20,
                parent_pid: Some(10),
                start_time: Some(child_start),
                ..Default::default()
            },
            ProcessInfo {
                pid: 30,
                parent_pid: Some(30),
                start_time: Some(parent_start),
                ..Default::default()
            },
            ProcessInfo {
                pid: 40,
                parent_pid: Some(10),
                start_time: Some(parent_start + Duration::from_secs(1)),
                ..Default::default()
            },
        ];

        retain_verified_parent_relations(&mut processes);

        assert_eq!(processes[1].parent_pid, None);
        assert_eq!(processes[2].parent_pid, None);
        assert_eq!(processes[3].parent_pid, Some(10));
    }
}
