//! Opt-in, read-only flight recording. Never used to authorize process termination.
pub mod cli;
pub mod report;
pub mod ui;
pub mod worker;

use crate::model::{ProcessInfo, ProcessSnapshot, SystemMemory};
use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 1;
pub const PREVIEW_LIMIT: usize = 600;
pub const EVENT_LIMIT: usize = 256;
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessKey {
    pub pid: u32,
    /// Decimal nanoseconds preserve Windows creation-time precision, including in JavaScript.
    pub started_unix_ns: String,
}

impl ProcessKey {
    pub fn of(process: &ProcessInfo) -> Option<Self> {
        Some(Self {
            pid: process.pid,
            started_unix_ns: process
                .start_time?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos()
                .to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    AiWorkloads,
    All,
    Tree {
        root: ProcessKey,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub mode: CaptureMode,
    pub interval_seconds: u64,
    pub duration_seconds: Option<u64>,
    pub max_log_bytes: u64,
    pub include_sensitive: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: CaptureMode::AiWorkloads,
            interval_seconds: 5,
            duration_seconds: None,
            max_log_bytes: 256 * 1024 * 1024,
            include_sensitive: false,
        }
    }
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            (1..=60).contains(&self.interval_seconds),
            "interval must be 1..60 seconds"
        );
        if let Some(seconds) = self.duration_seconds {
            ensure!(
                (1..=604_800).contains(&seconds),
                "duration must be 1..604800 seconds"
            );
        }
        ensure!(
            (1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&self.max_log_bytes),
            "log limit must be 1 MiB..4 GiB"
        );
        if let CaptureMode::Tree { root } = &self.mode {
            ensure!(
                root.pid > 0 && root.started_unix_ns.parse::<u128>().is_ok(),
                "invalid process identity"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub schema_version: u32,
    pub app_version: String,
    pub started_unix_ms: u64,
    pub config: Config,
}

impl Header {
    pub fn new(config: Config) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            started_unix_ms: unix_ms(),
            config,
        }
    }
}

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitiveMetadata {
    pub executable: Option<String>,
    pub command_line: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSample {
    pub pid: u32,
    pub identity: Option<ProcessKey>,
    pub parent_pid: Option<u32>,
    pub name: String,
    /// Heuristic hint, not an asserted application identity.
    pub workload_hint: String,
    pub ram_bytes: u64,
    pub vram_bytes: Option<u64>,
    /// 100% is one logical CPU; the first observation is null.
    pub cpu_percent: Option<f32>,
    pub io_read_bytes_total: Option<u64>,
    pub io_write_bytes_total: Option<u64>,
    pub io_read_bytes_delta: Option<u64>,
    pub io_write_bytes_delta: Option<u64>,
    pub listening_ports: Vec<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<SensitiveMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Totals {
    pub process_count: usize,
    /// Sum of working sets, not unique physical memory (shared pages may be counted twice).
    pub ram_bytes: u64,
    /// Only known per-process VRAM. None is not zero.
    pub vram_bytes_known: Option<u64>,
    pub vram_known_count: usize,
    pub cpu_percent_known: Option<f32>,
    pub cpu_known_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub sequence: u64,
    pub elapsed_ms: u64,
    pub observed_unix_ms: u64,
    pub collection_ms: u64,
    pub totals: Totals,
    pub system_memory: Option<SystemMemory>,
    pub vram_status: String,
    pub listener_status: String,
    pub processes: Vec<ProcessSample>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub elapsed_ms: u64,
    pub kind: String,
    pub message: String,
    /// For a disappearance, this is the last observed sample, not the process exit code.
    pub process: Option<ProcessSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub elapsed_ms: u64,
    pub collection_ms: u64,
    pub totals: Totals,
    pub system_memory: Option<SystemMemory>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub sample_count: u64,
    pub peak_ram_bytes: u64,
    pub peak_vram_bytes_known: Option<u64>,
    pub slow_sample_count: u64,
    pub omitted_preview_points: u64,
    pub omitted_events: u64,
    pub points: VecDeque<Point>,
    pub events: VecDeque<Event>,
}

impl Summary {
    pub fn push(&mut self, frame: &Frame, interval_ms: u64) {
        self.sample_count += 1;
        self.peak_ram_bytes = self.peak_ram_bytes.max(frame.totals.ram_bytes);
        if let Some(value) = frame.totals.vram_bytes_known {
            self.peak_vram_bytes_known = Some(self.peak_vram_bytes_known.unwrap_or(0).max(value));
        }
        self.slow_sample_count += u64::from(frame.collection_ms > interval_ms);
        if self.points.len() == PREVIEW_LIMIT {
            self.points.pop_front();
            self.omitted_preview_points += 1;
        }
        self.points.push_back(Point {
            elapsed_ms: frame.elapsed_ms,
            collection_ms: frame.collection_ms,
            totals: frame.totals.clone(),
            system_memory: frame.system_memory.clone(),
        });
        for event in &frame.events {
            if self.events.len() == EVENT_LIMIT {
                self.events.pop_front();
                self.omitted_events += 1;
            }
            self.events.push_back(event.clone());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", content = "data", rename_all = "snake_case")]
pub enum Record {
    Header(Header),
    Frame(Box<Frame>),
    End { reason: String },
}

pub struct Tracker {
    config: Config,
    sequence: u64,
    previous: HashMap<ProcessKey, ProcessSample>,
    retained_tree: HashSet<ProcessKey>,
    pressure_active: bool,
}

impl Tracker {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            sequence: 0,
            previous: HashMap::new(),
            retained_tree: HashSet::new(),
            pressure_active: false,
        }
    }

    pub fn sample(
        &mut self,
        snapshot: ProcessSnapshot,
        elapsed_ms: u64,
        collection_ms: u64,
    ) -> Frame {
        let all = &snapshot.processes;
        let keys: HashMap<_, _> = all
            .iter()
            .filter_map(|p| ProcessKey::of(p).map(|k| (p.pid, k)))
            .collect();
        let mut chosen: HashSet<u32> = match &self.config.mode {
            CaptureMode::All => all.iter().map(|p| p.pid).collect(),
            CaptureMode::AiWorkloads => all
                .iter()
                .filter(|p| is_ai_workload(p))
                .map(|p| p.pid)
                .collect(),
            CaptureMode::Tree { root } => {
                self.retained_tree.retain(|k| keys.get(&k.pid) == Some(k));
                self.retained_tree.insert(root.clone());
                self.retained_tree
                    .iter()
                    .filter(|k| keys.get(&k.pid) == Some(*k))
                    .map(|k| k.pid)
                    .collect()
            }
        };
        // Verified parent relationships only. Retain children when the original parent disappears.
        if !matches!(self.config.mode, CaptureMode::All) {
            let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
            for process in all {
                if let Some(parent) = process.parent_pid {
                    if let (Some(child_key), Some(parent_key)) =
                        (keys.get(&process.pid), keys.get(&parent))
                    {
                        if child_key.started_unix_ns.parse::<u128>().ok()
                            >= parent_key.started_unix_ns.parse::<u128>().ok()
                        {
                            children.entry(parent).or_default().push(process.pid);
                        }
                    }
                }
            }
            let mut queue: VecDeque<_> = chosen.iter().copied().collect();
            while let Some(parent) = queue.pop_front() {
                for &child in children.get(&parent).into_iter().flatten() {
                    if chosen.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
        }
        if matches!(self.config.mode, CaptureMode::Tree { .. }) {
            self.retained_tree = chosen
                .iter()
                .filter_map(|pid| keys.get(pid).cloned())
                .collect();
        }
        let mut processes: Vec<_> = all
            .iter()
            .filter(|p| chosen.contains(&p.pid))
            .map(|process| {
                let identity = ProcessKey::of(process);
                let previous = identity.as_ref().and_then(|key| self.previous.get(key));
                let telemetry = process.telemetry.as_ref();
                let read = telemetry.map(|t| t.read_bytes_total);
                let write = telemetry.map(|t| t.write_bytes_total);
                let mut ports: Vec<_> = process.local_endpoints.iter().map(|e| e.port).collect();
                ports.sort_unstable();
                ports.dedup();
                ProcessSample {
                    pid: process.pid,
                    identity,
                    parent_pid: process.parent_pid,
                    name: clipped(&process.name, 256),
                    workload_hint: workload_hint(process).into(),
                    ram_bytes: process.ram_bytes,
                    vram_bytes: process.vram_bytes(),
                    cpu_percent: telemetry
                        .and_then(|t| t.cpu_percent)
                        .filter(|v| v.is_finite() && *v >= 0.0),
                    io_read_bytes_total: read,
                    io_write_bytes_total: write,
                    io_read_bytes_delta: read
                        .and_then(|v| v.checked_sub(previous?.io_read_bytes_total?)),
                    io_write_bytes_delta: write
                        .and_then(|v| v.checked_sub(previous?.io_write_bytes_total?)),
                    listening_ports: ports,
                    sensitive: self.config.include_sensitive.then(|| SensitiveMetadata {
                        executable: process.exe_path.as_ref().map(|s| clipped(s, 4096)),
                        command_line: process.command_line.as_ref().map(|s| clipped(s, 4096)),
                        cwd: process.cwd.as_ref().map(|s| clipped(s, 4096)),
                    }),
                }
            })
            .collect();
        processes.sort_by_key(|p| p.pid);
        let mut totals = Totals::default();
        let mut events = Vec::new();
        let current: HashMap<_, _> = processes
            .iter()
            .filter_map(|p| p.identity.clone().map(|k| (k, p.clone())))
            .collect();
        for process in &processes {
            totals.process_count += 1;
            totals.ram_bytes = totals.ram_bytes.saturating_add(process.ram_bytes);
            if let Some(vram) = process.vram_bytes {
                totals.vram_bytes_known =
                    Some(totals.vram_bytes_known.unwrap_or(0).saturating_add(vram));
                totals.vram_known_count += 1;
            }
            if let Some(cpu) = process.cpu_percent {
                totals.cpu_percent_known = Some(totals.cpu_percent_known.unwrap_or(0.0) + cpu);
                totals.cpu_known_count += 1;
            }
            if let Some(key) = &process.identity {
                let kind = match self.previous.get(key) {
                    None => Some("first_seen"),
                    Some(old)
                        if process.ram_bytes.saturating_sub(old.ram_bytes) >= 256 * 1024 * 1024 =>
                    {
                        Some("ram_growth")
                    }
                    _ => None,
                };
                if let Some(kind) = kind {
                    events.push(Event {
                        elapsed_ms,
                        kind: kind.into(),
                        message: if kind == "first_seen" {
                            "First observed in this recording scope; not necessarily just started."
                                .into()
                        } else {
                            "Working set increased by at least 256 MiB since the previous sample."
                                .into()
                        },
                        process: Some(process.clone()),
                    });
                }
            }
        }
        for (key, previous) in &self.previous {
            if !current.contains_key(key) {
                events.push(Event { elapsed_ms, kind: "no_longer_observed".into(),
                    message: "No longer observed in scope; exit, access change or collection gap. This does not prove a crash.".into(),
                    process: Some(previous.clone()) });
            }
        }
        events.sort_by(|a, b| {
            a.process
                .as_ref()
                .map(|p| p.pid)
                .cmp(&b.process.as_ref().map(|p| p.pid))
                .then(a.kind.cmp(&b.kind))
        });
        let pressure = snapshot
            .system_memory
            .as_ref()
            .is_some_and(|m| m.total_bytes > 0 && m.available_bytes < m.total_bytes / 10);
        if pressure && !self.pressure_active {
            events.push(Event { elapsed_ms, kind: "system_memory_pressure".into(),
                message: "Available system memory is below 10%; correlation only, not proof of an out-of-memory failure.".into(), process: None });
        }
        self.pressure_active = pressure;
        self.previous = current;
        let sequence = self.sequence;
        self.sequence += 1;
        Frame {
            sequence,
            elapsed_ms,
            observed_unix_ms: unix_ms(),
            collection_ms,
            totals,
            system_memory: snapshot.system_memory,
            vram_status: clipped(&snapshot.vram_status, 1024),
            listener_status: clipped(&snapshot.listener_status, 1024),
            processes,
            events,
        }
    }
}

pub fn clipped(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub fn workload_hint(process: &ProcessInfo) -> &'static str {
    let text = format!(
        "{} {} {}",
        process.name,
        process.exe_path.as_deref().unwrap_or_default(),
        process.command_line.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    if text.contains("comfyui") {
        "ComfyUI"
    } else if text.contains("forge") || text.contains("stable-diffusion-webui") {
        "Forge/WebUI"
    } else if text.contains("ollama") {
        "Ollama"
    } else if text.contains("llama-server") || text.contains("llama.cpp") {
        "llama.cpp"
    } else if text.contains("train") || text.contains("accelerate") || text.contains("kohya") {
        "Training candidate"
    } else if process.python_related || process.name.to_lowercase().contains("python") {
        "Python"
    } else if process.is_gpu_active() {
        "GPU workload"
    } else {
        "Other"
    }
}

fn is_ai_workload(process: &ProcessInfo) -> bool {
    workload_hint(process) != "Other"
}

pub fn root_for_pid(snapshot: &ProcessSnapshot, pid: u32) -> anyhow::Result<ProcessKey> {
    match snapshot
        .processes
        .iter()
        .find(|p| p.pid == pid)
        .and_then(ProcessKey::of)
    {
        Some(key) => Ok(key),
        None => bail!("PID {pid} is absent or its creation time cannot be verified"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GpuProcessInfo, ProcessTelemetry};
    use std::time::Duration;

    fn p(pid: u32, ticks: u64, parent: Option<u32>, ram: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: "python.exe".into(),
            parent_pid: parent,
            start_time: Some(UNIX_EPOCH + Duration::from_nanos(ticks)),
            ram_bytes: ram,
            ..Default::default()
        }
    }
    fn snapshot(processes: Vec<ProcessInfo>) -> ProcessSnapshot {
        ProcessSnapshot {
            processes,
            ..Default::default()
        }
    }
    fn all() -> Tracker {
        Tracker::new(Config {
            mode: CaptureMode::All,
            ..Default::default()
        })
    }

    #[test]
    fn tree_survives_root_exit_but_never_follows_reused_pid() {
        let root = p(10, 100, None, 1);
        let mut tracker = Tracker::new(Config {
            mode: CaptureMode::Tree {
                root: ProcessKey::of(&root).unwrap(),
            },
            ..Default::default()
        });
        let first = tracker.sample(snapshot(vec![root, p(20, 200, Some(10), 2)]), 0, 1);
        assert_eq!(first.totals.process_count, 2);
        let next = tracker.sample(
            snapshot(vec![
                p(10, 300, None, 9),
                p(20, 200, None, 2),
                p(30, 400, Some(20), 3),
            ]),
            1000,
            1,
        );
        assert_eq!(
            next.processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![20, 30]
        );
        assert_eq!(
            next.events
                .iter()
                .filter(|e| e.kind == "no_longer_observed")
                .count(),
            1
        );
    }
    #[test]
    fn pid_reuse_within_one_millisecond_does_not_share_baseline() {
        let mut tracker = all();
        tracker.sample(snapshot(vec![p(1, 100, None, 99)]), 0, 0);
        let frame = tracker.sample(snapshot(vec![p(1, 200, None, 10)]), 1000, 0);
        assert!(frame.events.iter().any(|e| e.kind == "first_seen"));
        assert_eq!(
            frame
                .events
                .iter()
                .find(|e| e.kind == "no_longer_observed")
                .unwrap()
                .process
                .as_ref()
                .unwrap()
                .ram_bytes,
            99
        );
    }
    #[test]
    fn unknown_vram_is_not_zero_and_sensitive_metadata_is_opt_in() {
        let mut process = p(1, 100, None, 5);
        process.command_line = Some("secret-token".into());
        let frame = all().sample(snapshot(vec![process.clone()]), 0, 0);
        assert!(frame.totals.vram_bytes_known.is_none());
        assert!(!serde_json::to_string(&frame)
            .unwrap()
            .contains("secret-token"));
        process.gpu = Some(GpuProcessInfo {
            vram_bytes: Some(0),
            ..Default::default()
        });
        let frame = all().sample(snapshot(vec![process]), 0, 0);
        assert_eq!(frame.totals.vram_bytes_known, Some(0));
    }
    #[test]
    fn io_delta_requires_verified_identity_and_handles_counter_reset() {
        let mut tracker = all();
        let mut process = p(1, 100, None, 1);
        process.telemetry = Some(ProcessTelemetry {
            read_bytes_total: 100,
            ..Default::default()
        });
        assert!(tracker
            .sample(snapshot(vec![process.clone()]), 0, 0)
            .processes[0]
            .io_read_bytes_delta
            .is_none());
        process.telemetry.as_mut().unwrap().read_bytes_total = 150;
        assert_eq!(
            tracker
                .sample(snapshot(vec![process.clone()]), 1000, 0)
                .processes[0]
                .io_read_bytes_delta,
            Some(50)
        );
        process.telemetry.as_mut().unwrap().read_bytes_total = 1;
        assert!(
            tracker.sample(snapshot(vec![process]), 2000, 0).processes[0]
                .io_read_bytes_delta
                .is_none()
        );
    }
    #[test]
    fn summary_memory_is_bounded_and_preserves_all_time_peaks() {
        let mut tracker = all();
        let mut summary = Summary::default();
        for n in 0..5000 {
            let frame = tracker.sample(snapshot(vec![p(1, (n + 1) * 100, None, 5000 - n)]), n, 0);
            summary.push(&frame, 1000);
        }
        assert_eq!(summary.points.len(), PREVIEW_LIMIT);
        assert_eq!(summary.events.len(), EVENT_LIMIT);
        assert_eq!(summary.peak_ram_bytes, 5000);
        assert_eq!(summary.sample_count, 5000);
        assert_eq!(summary.omitted_events, 9999 - EVENT_LIMIT as u64);
        assert_eq!(summary.omitted_preview_points, 5000 - PREVIEW_LIMIT as u64);
    }
    #[test]
    fn config_rejects_unbounded_or_busy_loop_inputs() {
        assert!(Config {
            interval_seconds: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Config {
            max_log_bytes: u64::MAX,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Config::default().validate().is_ok());
    }
}
