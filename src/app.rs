use crate::collectors::process_collector;
use crate::model::{ProcessInfo, ProcessSnapshot, SnapshotState, SortPreset};
use crate::services::process_identity;
use crate::settings::{Settings, TableView};
use crate::ui;
use anyhow::anyhow;
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

type LoadResult = anyhow::Result<(ProcessSnapshot, Settings)>;

struct LoadRequest {
    settings: Settings,
    response: mpsc::Sender<LoadResult>,
    repaint: egui::Context,
}

static LOAD_WORKER: OnceLock<Result<mpsc::SyncSender<LoadRequest>, String>> = OnceLock::new();

pub struct WatcherApp {
    pub settings: Settings,
    pub settings_path: PathBuf,
    pub processes: Vec<ProcessInfo>,
    pub sort: SortPreset,
    pub search: String,
    pub status: String,
    pub vram_status: String,
    pub listener_status: String,
    pub timing_status: String,
    pub snapshot_delta: Option<SnapshotDeltaSummary>,
    pub last_updated: Option<SystemTime>,
    pub loading: bool,
    pub selected_pid: Option<u32>,
    pub pending_action: Option<PendingAction>,
    pub show_settings: bool,
    pub detail_panel_height: f32,
    pub protected_process_names_text: String,
    pub python_keywords_text: String,
    pub codex_root_keywords_text: String,
    pub focus_search_requested: bool,
    pub settings_dirty: bool,
    load_receiver: Option<Receiver<LoadResult>>,
    last_auto_refresh: Instant,
    scheduled_reload_at: Option<Instant>,
    table_scroll_request: Option<usize>,
    screenshot_path: Option<PathBuf>,
    screenshot_receiver: Option<Receiver<anyhow::Result<PathBuf>>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotDeltaSummary {
    pub started: usize,
    pub exited: usize,
    pub changed: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VisibleProcessStats {
    pub rows: usize,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub gpu_processes: usize,
    pub local_web_processes: usize,
}

impl VisibleProcessStats {
    fn add(&mut self, process: &ProcessInfo) {
        self.rows += 1;
        self.ram_bytes = self.ram_bytes.saturating_add(process.ram_bytes);
        if let Some(vram) = process.vram_bytes() {
            self.vram_bytes = self.vram_bytes.saturating_add(vram);
        }
        self.gpu_processes += usize::from(process.is_gpu_active());
        self.local_web_processes += usize::from(!process.local_endpoints.is_empty());
    }
}

#[derive(Debug, Clone, Default)]
pub struct VisibleProcessView {
    pub indices: Vec<usize>,
    pub stats: VisibleProcessStats,
}

#[derive(Debug, Clone)]
pub enum PendingAction {
    Close { targets: Vec<ProcessInfo> },
    Kill { targets: Vec<ProcessInfo> },
    KillTree { targets: Vec<ProcessInfo> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickFilter {
    All,
    Python,
    GpuActive,
    LocalWeb,
    CodexTerminal,
    HeavyRam,
    HeavyVram,
    Changed,
}

impl Default for WatcherApp {
    fn default() -> Self {
        let settings_path = Settings::default_path();
        let (settings, status) = match Settings::load_or_default(&settings_path) {
            Ok(settings) => (settings, "Ready. Click Load.".to_string()),
            Err(error) => (
                Settings::default(),
                format!("Failed to load settings.json; using defaults: {error}"),
            ),
        };
        let sort = SortPreset::from_settings(&settings.default_sort);
        let protected_process_names_text = lines_from_vec(&settings.protected_process_names);
        let python_keywords_text = lines_from_vec(&settings.python_keywords);
        let codex_root_keywords_text = lines_from_vec(&settings.codex_root_keywords);

        Self {
            settings,
            settings_path,
            processes: Vec::new(),
            sort,
            search: String::new(),
            status,
            vram_status: String::new(),
            listener_status: String::new(),
            timing_status: String::new(),
            snapshot_delta: None,
            last_updated: None,
            loading: false,
            selected_pid: None,
            pending_action: None,
            show_settings: false,
            detail_panel_height: 240.0,
            protected_process_names_text,
            python_keywords_text,
            codex_root_keywords_text,
            focus_search_requested: false,
            settings_dirty: false,
            load_receiver: None,
            last_auto_refresh: Instant::now(),
            scheduled_reload_at: None,
            table_scroll_request: None,
            screenshot_path: None,
            screenshot_receiver: None,
        }
    }
}

fn load_worker() -> anyhow::Result<&'static mpsc::SyncSender<LoadRequest>> {
    let worker = LOAD_WORKER.get_or_init(|| {
        let (request_sender, request_receiver) = mpsc::sync_channel::<LoadRequest>(1);
        thread::Builder::new()
            .name("runscope-snapshot".to_string())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let settings = request.settings;
                    let result = process_collector::collect_processes(&settings)
                        .map(|snapshot| (snapshot, settings));
                    let _ = request.response.send(result);
                    request.repaint.request_repaint();
                }
            })
            .map(|_| request_sender)
            .map_err(|error| error.to_string())
    });
    worker
        .as_ref()
        .map_err(|error| anyhow!("failed to start snapshot worker: {error}"))
}

impl eframe::App for WatcherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_screenshot_result();
        self.handle_keyboard_shortcuts(ctx);
        self.poll_load();
        self.maybe_auto_refresh(ctx);
        ui::draw(ctx, self);
        self.maybe_add_screenshot_callback(ctx);
    }
}

impl WatcherApp {
    pub fn set_screenshot_path(&mut self, path: PathBuf) {
        self.screenshot_path = Some(path);
    }

    pub fn start_load(&mut self, ctx: &egui::Context) {
        if self.loading {
            return;
        }

        self.last_auto_refresh = Instant::now();
        self.scheduled_reload_at = None;
        let settings = self.settings.clone();
        let (response, receiver) = mpsc::channel();
        let request = LoadRequest {
            settings,
            response,
            repaint: ctx.clone(),
        };
        let worker = match load_worker() {
            Ok(worker) => worker,
            Err(error) => {
                self.status = format!("Load worker unavailable: {error}");
                return;
            }
        };
        match worker.try_send(request) {
            Ok(()) => {
                self.loading = true;
                self.status = "Loading process snapshot...".to_string();
                self.load_receiver = Some(receiver);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.status = "Load worker is still busy.".to_string();
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.status = "Load worker disconnected.".to_string();
            }
        }
    }

    pub fn set_sort(&mut self, sort: SortPreset) {
        self.sort = sort;
        self.settings.default_sort = sort.as_settings_value().to_string();
        sort_processes(&mut self.processes, self.sort);
        self.scroll_selection_into_view();
        self.save_settings_quietly();
    }

    pub fn set_table_view(&mut self, table_view: TableView) {
        self.settings.table_view = table_view;
        self.save_settings_quietly();
    }

    pub fn set_auto_refresh_enabled(&mut self, enabled: bool) {
        self.settings.set_auto_refresh_enabled(enabled);
        self.last_auto_refresh = Instant::now();
        self.save_settings_quietly();
    }

    pub fn visible_process_indices(&self) -> Vec<usize> {
        self.visible_process_view().indices
    }

    pub fn visible_process_view(&self) -> VisibleProcessView {
        let query = SearchQuery::parse(&self.search);
        let mut view = VisibleProcessView {
            indices: Vec::with_capacity(self.processes.len()),
            ..Default::default()
        };
        for (index, process) in self.processes.iter().enumerate() {
            if !self.process_matches_filters(process, &query) {
                continue;
            }
            view.indices.push(index);
            view.stats.add(process);
        }
        view
    }

    pub fn visible_process_stats(&self) -> VisibleProcessStats {
        let query = SearchQuery::parse(&self.search);
        let mut stats = VisibleProcessStats::default();
        for process in &self.processes {
            if self.process_matches_filters(process, &query) {
                stats.add(process);
            }
        }
        stats
    }

    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        let pid = self.selected_pid?;
        let process = self.processes.iter().find(|process| process.pid == pid)?;
        let query = SearchQuery::parse(&self.search);
        self.process_matches_filters(process, &query)
            .then_some(process)
    }

    pub fn selected_process_summary(&self) -> Option<String> {
        self.selected_process().map(process_summary)
    }

    pub fn take_search_focus_request(&mut self) -> bool {
        let requested = self.focus_search_requested;
        self.focus_search_requested = false;
        requested
    }

    pub fn take_table_scroll_request(&mut self) -> Option<usize> {
        self.table_scroll_request.take()
    }

    pub fn select_pid(&mut self, pid: u32) {
        let rows = self.visible_process_indices();
        if let Some(position) = rows
            .iter()
            .position(|index| self.processes[*index].pid == pid)
        {
            self.selected_pid = Some(pid);
            self.table_scroll_request = Some(position);
        }
    }

    pub fn apply_quick_filter(&mut self, filter: QuickFilter) {
        toggle_quick_filter(&mut self.settings, filter);
        self.scroll_selection_into_view();
        self.save_settings_quietly();
    }

    pub fn quick_filter_active(&self, filter: QuickFilter) -> bool {
        match filter {
            QuickFilter::All => !self.has_active_quick_filters(),
            QuickFilter::Python => self.settings.python_only,
            QuickFilter::GpuActive => self.settings.gpu_active_only,
            QuickFilter::LocalWeb => self.settings.local_web_only,
            QuickFilter::CodexTerminal => self.settings.codex_related_only,
            QuickFilter::HeavyRam => self.settings.heavy_ram_only,
            QuickFilter::HeavyVram => self.settings.heavy_vram_only,
            QuickFilter::Changed => self.settings.memory_changed_only,
        }
    }

    pub fn has_active_quick_filters(&self) -> bool {
        self.settings.python_only
            || self.settings.gpu_active_only
            || self.settings.local_web_only
            || self.settings.codex_related_only
            || self.settings.heavy_ram_only
            || self.settings.heavy_vram_only
            || self.settings.memory_changed_only
    }

    pub fn copy_visible_tsv(&mut self, ctx: &egui::Context) {
        let rows = self.visible_process_indices();
        let mut lines = Vec::with_capacity(rows.len() + 1);
        lines.push(
            "State\tScope\tPID\tProcess Name\tRAM MB\tRAM Delta MB\tVRAM MB\tVRAM Delta MB\tAge\tParent PID\tParent Name\tLocal Web\tExecutable Path\tCommand Line"
                .to_string(),
        );
        for index in rows {
            let process = &self.processes[index];
            lines.push(
                [
                    process.snapshot_state.to_string(),
                    process.scope.to_string(),
                    process.pid.to_string(),
                    process.name.clone(),
                    crate::services::formatter::bytes_to_mb_text(process.ram_bytes),
                    crate::services::formatter::optional_delta_mb_text(process.ram_delta_bytes),
                    crate::services::formatter::optional_bytes_to_mb_text(process.vram_bytes()),
                    crate::services::formatter::optional_delta_mb_text(process.vram_delta_bytes),
                    crate::services::formatter::age_text(process.start_time),
                    process
                        .parent_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_default(),
                    process.parent_name.clone().unwrap_or_default(),
                    process.local_web_summary(),
                    process.exe_path.clone().unwrap_or_default(),
                    process.command_line.clone().unwrap_or_default(),
                ]
                .into_iter()
                .map(|value| sanitize_tsv_field(&value))
                .collect::<Vec<_>>()
                .join("\t"),
            );
        }
        let count = lines.len().saturating_sub(1);
        ctx.copy_text(lines.join("\r\n"));
        self.status = format!("Copied {count} visible process row(s) as TSV.");
    }

    pub fn copy_visible_pids(&mut self, ctx: &egui::Context) {
        let pids = self
            .visible_process_indices()
            .into_iter()
            .map(|index| self.processes[index].pid.to_string())
            .collect::<Vec<_>>();
        let count = pids.len();
        ctx.copy_text(pids.join("\r\n"));
        self.status = format!("Copied {count} visible PID(s).");
    }

    pub fn copy_diagnostics(&mut self, ctx: &egui::Context) {
        let stats = self.visible_process_stats();
        let text = [
            format!("RunScope {}", env!("CARGO_PKG_VERSION")),
            format!("Architecture: {}", std::env::consts::ARCH),
            format!(
                "Processes: {} total, {} visible",
                self.processes.len(),
                stats.rows
            ),
            format!("Sort: {}", self.sort.display_name()),
            format!("Table view: {}", self.settings.table_view),
            format!(
                "Refresh: {} ({} ms)",
                if self.settings.auto_refresh_enabled() {
                    "auto"
                } else {
                    "manual"
                },
                self.settings.auto_refresh_interval_ms
            ),
            format!("VRAM backend: {}", empty_as_unavailable(&self.vram_status)),
            format!(
                "Local listeners: {}",
                empty_as_unavailable(&self.listener_status)
            ),
            format!("Timing: {}", empty_as_unavailable(&self.timing_status)),
            format!("Settings: {}", self.settings_path.to_string_lossy()),
        ]
        .join("\r\n");
        ctx.copy_text(text);
        self.status = "Copied diagnostics.".to_string();
    }

    pub fn copy_selected_json(&mut self, ctx: &egui::Context) {
        let Some(process) = self.selected_process().cloned() else {
            self.status = "No process selected.".to_string();
            return;
        };
        let value = serde_json::json!({
            "snapshot_state": process.snapshot_state.to_string(),
            "scope": process.scope.to_string(),
            "pid": process.pid,
            "name": process.name,
            "ram_bytes": process.ram_bytes,
            "ram_delta_bytes": process.ram_delta_bytes,
            "vram_bytes": process.vram_bytes(),
            "vram_delta_bytes": process.vram_delta_bytes,
            "virtual_memory_bytes": process.virtual_memory_bytes,
            "parent_pid": process.parent_pid,
            "parent_name": process.parent_name,
            "executable_path": process.exe_path,
            "command_line": process.command_line,
            "working_directory": process.cwd,
            "start_time_unix_seconds": process.start_time.and_then(|time| {
                time.duration_since(std::time::UNIX_EPOCH).ok().map(|duration| duration.as_secs())
            }),
            "protected": process.protected,
            "protection_reason": process.protection_reason,
            "python_related": process.python_related,
            "codex_related": process.codex_related,
            "local_endpoints": process.local_endpoints.iter().map(|endpoint| serde_json::json!({
                "bind_address": endpoint.bind_address,
                "port": endpoint.port,
                "url": endpoint.url,
            })).collect::<Vec<_>>(),
            "gpu": process.gpu.as_ref().map(|gpu| serde_json::json!({
                "device_indices": gpu.device_indices,
                "device_names": gpu.device_names,
                "vram_bytes": gpu.vram_bytes,
                "process_type": format!("{:?}", gpu.process_type),
            })),
        });
        match serde_json::to_string_pretty(&value) {
            Ok(text) => {
                ctx.copy_text(text);
                self.status = format!("Copied PID {} as JSON.", process.pid);
            }
            Err(error) => {
                self.status = format!("Failed to serialize selected process: {error}");
            }
        }
    }

    pub fn move_selection(&mut self, amount: isize) {
        let rows = self.visible_process_indices();
        if rows.is_empty() {
            self.selected_pid = None;
            return;
        }
        let current = self.selected_pid.and_then(|pid| {
            rows.iter()
                .position(|index| self.processes[*index].pid == pid)
        });
        let next = match current {
            Some(current) => current
                .saturating_add_signed(amount)
                .min(rows.len().saturating_sub(1)),
            None if amount < 0 => rows.len() - 1,
            None => 0,
        };
        self.selected_pid = Some(self.processes[rows[next]].pid);
        self.table_scroll_request = Some(next);
    }

    pub fn move_selection_to_edge(&mut self, end: bool) {
        let rows = self.visible_process_indices();
        if rows.is_empty() {
            self.selected_pid = None;
            return;
        }
        let position = if end { rows.len() - 1 } else { 0 };
        self.selected_pid = Some(self.processes[rows[position]].pid);
        self.table_scroll_request = Some(position);
    }

    fn scroll_selection_into_view(&mut self) {
        let Some(pid) = self.selected_pid else {
            return;
        };
        let rows = self.visible_process_indices();
        self.table_scroll_request = rows
            .iter()
            .position(|index| self.processes[*index].pid == pid);
    }

    pub fn open_selected_local_web(&mut self, ctx: &egui::Context) {
        let Some(process) = self.selected_process() else {
            self.status = "No process selected.".to_string();
            return;
        };
        let Some(url) = process.primary_local_web_url().map(str::to_string) else {
            self.status = format!("PID {} has no Local Web candidate.", process.pid);
            return;
        };
        ctx.open_url(egui::OpenUrl::new_tab(&url));
        self.status = format!("Opened {url}.");
    }

    pub fn copy_selected_summary(&mut self, ctx: &egui::Context) {
        let Some(summary) = self.selected_process_summary() else {
            self.status = "No process selected.".to_string();
            return;
        };
        ctx.copy_text(summary);
        self.status = "Copied selected process summary.".to_string();
    }

    pub fn reveal_selected_executable(&mut self) {
        let Some(process) = self.selected_process() else {
            self.status = "No process selected.".to_string();
            return;
        };
        let Some(path) = process.exe_path.as_deref() else {
            self.status = format!("PID {} has no executable path.", process.pid);
            return;
        };
        match explorer_select(path).spawn() {
            Ok(_) => {
                self.status = format!("Selected executable in Explorer for PID {}.", process.pid);
            }
            Err(error) => {
                self.status = format!("Failed to select executable in Explorer: {error}");
            }
        }
    }

    pub fn open_selected_executable_folder(&mut self) {
        let Some(process) = self.selected_process() else {
            self.status = "No process selected.".to_string();
            return;
        };
        let Some(path) = process.exe_path.as_deref() else {
            self.status = format!("PID {} has no executable path.", process.pid);
            return;
        };
        let Some(parent) = Path::new(path).parent() else {
            self.status = format!("PID {} executable has no parent folder.", process.pid);
            return;
        };
        match Command::new("explorer.exe").arg(parent).spawn() {
            Ok(_) => {
                self.status = format!("Opened executable folder for PID {}.", process.pid);
            }
            Err(error) => {
                self.status = format!("Failed to open executable folder: {error}");
            }
        }
    }

    pub fn open_selected_working_directory(&mut self) {
        let Some(process) = self.selected_process() else {
            self.status = "No process selected.".to_string();
            return;
        };
        let Some(cwd) = process.cwd.as_deref() else {
            self.status = format!("PID {} has no working directory.", process.pid);
            return;
        };
        let pid = process.pid;
        match Command::new("explorer.exe").arg(cwd).spawn() {
            Ok(_) => {
                self.status = format!("Opened working directory for PID {pid}.");
            }
            Err(error) => {
                self.status = format!("Failed to open working directory: {error}");
            }
        }
    }

    pub fn request_kill_selected(&mut self) {
        let Some(process) = self.selected_process().cloned() else {
            self.status = "No process selected.".to_string();
            return;
        };
        if process.protected {
            self.status = format!("PID {} is protected.", process.pid);
            return;
        }
        self.pending_action = Some(PendingAction::Kill {
            targets: vec![process],
        });
    }

    pub fn request_kill_tree_selected(&mut self) {
        let Some(root) = self.selected_process().cloned() else {
            self.status = "No process selected.".to_string();
            return;
        };
        if root.protected {
            self.status = format!("PID {} is protected.", root.pid);
            return;
        }
        let mut current_processes =
            match process_collector::collect_processes_for_action(&self.settings) {
                Ok(processes) => processes,
                Err(error) => {
                    self.status = format!("Kill Tree pre-check failed: {error}");
                    return;
                }
            };
        enrich_action_processes(&mut current_processes, &self.processes);
        let Some(current_root) = current_processes
            .iter()
            .find(|candidate| candidate.pid == root.pid)
        else {
            self.status = format!("PID {} no longer exists. Reload and try again.", root.pid);
            return;
        };
        if let Err(error) = process_identity::ensure_same_process(&root, current_root) {
            self.status = error.to_string();
            return;
        }
        let targets = tree_targets_from(&current_processes, root.pid);
        if let Some(protected) = targets.iter().find(|process| process.protected) {
            self.status = format!(
                "Kill Tree blocked: protected PID {} ({}) is in the tree.",
                protected.pid, protected.name
            );
            return;
        }
        self.pending_action = Some(PendingAction::KillTree { targets });
    }

    pub fn request_close_selected(&mut self) {
        let Some(process) = self.selected_process().cloned() else {
            self.status = "No process selected.".to_string();
            return;
        };
        if process.protected {
            self.status = format!("PID {} is protected.", process.pid);
            return;
        }
        self.pending_action = Some(PendingAction::Close {
            targets: vec![process],
        });
    }

    pub fn confirm_pending_action(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_action.take() else {
            return;
        };

        match action {
            PendingAction::Close { targets } => {
                let Some(process) = targets.first() else {
                    self.status = "No process selected.".to_string();
                    return;
                };
                let current_processes =
                    match process_collector::collect_processes_for_action(&self.settings) {
                        Ok(processes) => processes,
                        Err(error) => {
                            self.status = format!("Close pre-check failed: {error}");
                            return;
                        }
                    };
                let Some(current) = current_processes
                    .iter()
                    .find(|candidate| candidate.pid == process.pid)
                else {
                    self.status = format!(
                        "PID {} no longer exists. Reload and try again.",
                        process.pid
                    );
                    return;
                };
                if let Err(error) = process_identity::ensure_same_process(process, current) {
                    self.status = error.to_string();
                    return;
                }
                match crate::services::terminator::close_process(current) {
                    Ok(count) => {
                        self.status = format!(
                            "WM_CLOSE sent to {count} window(s) for PID {}. Reload scheduled.",
                            process.pid
                        );
                        self.schedule_reload(ctx, Duration::from_millis(900));
                    }
                    Err(error) => {
                        self.status = format!("Close failed for PID {}: {error}", process.pid);
                    }
                }
            }
            PendingAction::Kill { targets } => {
                let Some(process) = targets.first() else {
                    self.status = "No process selected.".to_string();
                    return;
                };
                let current_processes =
                    match process_collector::collect_processes_for_action(&self.settings) {
                        Ok(processes) => processes,
                        Err(error) => {
                            self.status = format!("Kill pre-check failed: {error}");
                            return;
                        }
                    };
                let Some(current) = current_processes
                    .iter()
                    .find(|candidate| candidate.pid == process.pid)
                else {
                    self.status = format!(
                        "PID {} no longer exists. Reload and try again.",
                        process.pid
                    );
                    return;
                };
                if let Err(error) = process_identity::ensure_same_process(process, current) {
                    self.status = error.to_string();
                    return;
                }
                match crate::services::terminator::kill_process(current) {
                    Ok(()) => {
                        self.status = format!("Killed PID {}. Reload scheduled.", process.pid);
                        self.schedule_reload(ctx, Duration::from_millis(120));
                    }
                    Err(error) => {
                        self.status = format!("Kill failed for PID {}: {error}", process.pid);
                    }
                }
            }
            PendingAction::KillTree { targets } => {
                let Some(root) = targets.first() else {
                    self.status = "No process tree targets.".to_string();
                    return;
                };
                let root_pid = root.pid;
                let mut current_processes =
                    match process_collector::collect_processes_for_action(&self.settings) {
                        Ok(processes) => processes,
                        Err(error) => {
                            self.status = format!("Kill Tree pre-check failed: {error}");
                            return;
                        }
                    };
                enrich_action_processes(&mut current_processes, &self.processes);
                let Some(current_root) = current_processes
                    .iter()
                    .find(|candidate| candidate.pid == root_pid)
                else {
                    self.status = format!("PID {root_pid} no longer exists. Reload and try again.");
                    return;
                };
                if let Err(error) = process_identity::ensure_same_process(root, current_root) {
                    self.status = error.to_string();
                    return;
                }

                let current_targets = tree_targets_from(&current_processes, root_pid);
                if let Some(protected) = current_targets.iter().find(|process| process.protected) {
                    self.status = format!(
                        "Kill Tree blocked: protected PID {} ({}) is in the tree.",
                        protected.pid, protected.name
                    );
                    return;
                }
                if process_identity::target_list_changed(&targets, &current_targets) {
                    self.pending_action = Some(PendingAction::KillTree {
                        targets: current_targets,
                    });
                    self.status =
                        "Process tree changed. Review updated target list before confirming."
                            .to_string();
                    return;
                }

                match crate::services::terminator::kill_tree(&current_targets) {
                    Ok(killed) => {
                        self.status = format!(
                            "Killed process tree rooted at PID {root_pid}: {:?}. Reload scheduled.",
                            killed
                        );
                        self.schedule_reload(ctx, Duration::from_millis(120));
                    }
                    Err(error) => {
                        self.status = format!("Kill Tree failed for root PID {root_pid}: {error}");
                    }
                }
            }
        }
    }

    pub fn cancel_pending_action(&mut self) {
        self.pending_action = None;
    }

    pub fn save_settings_quietly(&mut self) -> bool {
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.status = format!("Failed to save settings.json: {error}");
            false
        } else {
            true
        }
    }

    pub fn open_settings_window(&mut self) {
        self.sync_settings_editor();
        self.settings_dirty = false;
        self.show_settings = true;
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    pub fn save_settings_from_editor(&mut self) {
        self.settings.protected_process_names = vec_from_lines(&self.protected_process_names_text);
        self.settings.python_keywords = vec_from_lines(&self.python_keywords_text);
        self.settings.codex_root_keywords = vec_from_lines(&self.codex_root_keywords_text);
        self.settings.default_sort = self.sort.as_settings_value().to_string();
        self.reclassify_current_processes();
        self.last_auto_refresh = Instant::now();
        if self.save_settings_quietly() {
            self.settings_dirty = false;
            self.status = "Settings saved.".to_string();
        }
    }

    pub fn reload_settings_from_disk(&mut self) {
        match Settings::load_or_default(&self.settings_path) {
            Ok(settings) => {
                self.settings = settings;
                self.sort = SortPreset::from_settings(&self.settings.default_sort);
                self.reclassify_current_processes();
                sort_processes(&mut self.processes, self.sort);
                self.sync_settings_editor();
                self.last_auto_refresh = Instant::now();
                self.settings_dirty = false;
                self.status = "Settings reloaded.".to_string();
            }
            Err(error) => {
                self.status = format!("Failed to reload settings.json: {error}");
            }
        }
    }

    pub fn reset_settings_to_default(&mut self) {
        self.settings = Settings::default();
        self.sort = SortPreset::from_settings(&self.settings.default_sort);
        self.reclassify_current_processes();
        sort_processes(&mut self.processes, self.sort);
        self.sync_settings_editor();
        self.last_auto_refresh = Instant::now();
        if self.save_settings_quietly() {
            self.settings_dirty = false;
            self.status = "Settings reset to defaults.".to_string();
        }
    }

    pub fn open_settings_json(&mut self) {
        if !self.settings_path.exists() && !self.save_settings_quietly() {
            return;
        }
        match Command::new("notepad.exe").arg(&self.settings_path).spawn() {
            Ok(_) => {
                self.status = format!("Opened {}.", self.settings_path.to_string_lossy());
            }
            Err(error) => {
                self.status = format!("Failed to open settings.json: {error}");
            }
        }
    }

    fn sync_settings_editor(&mut self) {
        self.protected_process_names_text = lines_from_vec(&self.settings.protected_process_names);
        self.python_keywords_text = lines_from_vec(&self.settings.python_keywords);
        self.codex_root_keywords_text = lines_from_vec(&self.settings.codex_root_keywords);
    }

    fn reclassify_current_processes(&mut self) {
        crate::services::scope_detector::assign_scopes(&mut self.processes, &self.settings);
        for process in &mut self.processes {
            process.refresh_searchable_text();
        }
        self.scroll_selection_into_view();
    }

    fn poll_load(&mut self) {
        let mut received = None;
        if let Some(receiver) = &self.load_receiver {
            match receiver.try_recv() {
                Ok(result) => received = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    received = Some(Err(anyhow!("load worker disconnected")));
                }
            }
        }

        let Some(result) = received else {
            return;
        };

        self.load_receiver = None;
        self.loading = false;
        self.last_auto_refresh = Instant::now();
        match result {
            Ok((snapshot, loaded_settings)) => {
                let selected_identity = self.selected_pid.and_then(|pid| {
                    self.processes
                        .iter()
                        .find(|process| process.pid == pid)
                        .cloned()
                });
                let mut processes = snapshot.processes;
                self.snapshot_delta = apply_snapshot_deltas(
                    &self.processes,
                    &mut processes,
                    self.last_updated.is_some(),
                );
                if !classification_settings_equal(&loaded_settings, &self.settings) {
                    crate::services::scope_detector::assign_scopes(&mut processes, &self.settings);
                }
                for process in &mut processes {
                    process.refresh_searchable_text();
                }
                self.processes = processes;
                sort_processes(&mut self.processes, self.sort);
                self.last_updated = Some(SystemTime::now());
                self.vram_status = snapshot.vram_status;
                self.listener_status = snapshot.listener_status;
                self.timing_status = snapshot.timing_status;
                self.status = match self.snapshot_delta {
                    Some(delta) => format!(
                        "Loaded {} processes. Snapshot: +{} started, -{} exited, {} changed.",
                        self.processes.len(),
                        delta.started,
                        delta.exited,
                        delta.changed
                    ),
                    None => format!("Loaded {} processes.", self.processes.len()),
                };
                if let Some(expected) = selected_identity {
                    if !self.processes.iter().any(|process| {
                        process.pid == expected.pid
                            && process_identity::same_process_for_snapshot(&expected, process)
                    }) {
                        self.selected_pid = None;
                    }
                }
                if self.selected_pid.is_none() && self.screenshot_path.is_some() {
                    self.selected_pid = self
                        .visible_process_indices()
                        .first()
                        .map(|index| self.processes[*index].pid);
                }
            }
            Err(error) => {
                self.status = format!("Load failed: {error}");
            }
        }
    }

    fn maybe_auto_refresh(&mut self, ctx: &egui::Context) {
        if self.loading {
            return;
        }
        if let Some(deadline) = self.scheduled_reload_at {
            let now = Instant::now();
            if now >= deadline {
                self.start_load(ctx);
            } else {
                ctx.request_repaint_after(deadline.saturating_duration_since(now));
            }
            return;
        }
        if !self.settings.auto_refresh_enabled()
            || self.pending_action.is_some()
            || self.show_settings
        {
            return;
        }
        let interval = Duration::from_millis(self.settings.auto_refresh_interval_ms.max(1000));
        let elapsed = self.last_auto_refresh.elapsed();
        if elapsed >= interval {
            self.start_load(ctx);
        } else {
            ctx.request_repaint_after(interval - elapsed);
        }
    }

    fn schedule_reload(&mut self, ctx: &egui::Context, delay: Duration) {
        self.scheduled_reload_at = Some(Instant::now() + delay);
        ctx.request_repaint_after(delay);
    }

    fn maybe_add_screenshot_callback(&mut self, ctx: &egui::Context) {
        if self.loading || self.screenshot_receiver.is_some() {
            return;
        }
        let Some(path) = self.screenshot_path.take() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.screenshot_receiver = Some(receiver);
        let repaint = ctx.clone();
        let callback = eframe::egui_glow::CallbackFn::new(move |info, painter| {
            let image = painter.read_screen_rgba(info.screen_size_px);
            let result =
                crate::services::screenshot::save_bmp(&image, &path).map(|()| path.clone());
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("runscope_screenshot_callback"),
        ))
        .add(egui::PaintCallback {
            rect: ctx.screen_rect(),
            callback: std::sync::Arc::new(callback),
        });
    }

    fn poll_screenshot_result(&mut self) {
        let Some(receiver) = &self.screenshot_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(path)) => {
                self.status = format!("Saved screenshot to {}.", path.to_string_lossy());
                self.screenshot_receiver = None;
            }
            Ok(Err(error)) => {
                self.status = format!("Failed to save screenshot: {error}");
                self.screenshot_receiver = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.status =
                    "Failed to save screenshot: render callback disconnected.".to_string();
                self.screenshot_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn process_matches_filters(&self, process: &ProcessInfo, query: &SearchQuery) -> bool {
        if !self.settings.show_system_processes && process.protected {
            return false;
        }
        if self.settings.python_only && !process.python_related {
            return false;
        }
        if self.settings.gpu_active_only && !process.is_gpu_active() {
            return false;
        }
        if self.settings.codex_related_only && !process.codex_related {
            return false;
        }
        if self.settings.local_web_only && process.local_endpoints.is_empty() {
            return false;
        }
        if self.settings.heavy_ram_only
            && process.ram_bytes < mb_to_bytes(self.settings.heavy_ram_threshold_mb)
        {
            return false;
        }
        if self.settings.heavy_vram_only
            && process.vram_bytes().unwrap_or(0)
                < mb_to_bytes(self.settings.heavy_vram_threshold_mb)
        {
            return false;
        }
        if self.settings.memory_changed_only
            && !matches!(
                process.snapshot_state,
                SnapshotState::New | SnapshotState::Changed
            )
        {
            return false;
        }
        if !query.matches(process) {
            return false;
        }
        true
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if self.pending_action.is_some() {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.cancel_pending_action();
            }
            return;
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.start_load(ctx);
        }

        let reload_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::R);
        if ctx.input_mut(|input| input.consume_shortcut(&reload_shortcut)) {
            self.start_load(ctx);
        }

        let focus_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::F);
        if ctx.input_mut(|input| input.consume_shortcut(&focus_shortcut)) {
            self.focus_search_requested = true;
        }

        if ctx.wants_keyboard_input() {
            return;
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
            self.move_selection(1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            self.move_selection(-1);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::PageDown)) {
            self.move_selection(10);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::PageUp)) {
            self.move_selection(-10);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Home)) {
            self.move_selection_to_edge(false);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::End)) {
            self.move_selection_to_edge(true);
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if !self.search.is_empty() {
                self.search.clear();
                self.status = "Search cleared.".to_string();
            } else if self.has_active_quick_filters() {
                self.apply_quick_filter(QuickFilter::All);
                self.status = "Quick filter cleared.".to_string();
            }
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            self.open_selected_local_web(ctx);
        }

        let copy_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::C);
        if ctx.input_mut(|input| input.consume_shortcut(&copy_shortcut)) {
            self.copy_selected_summary(ctx);
        }

        let copy_table_shortcut = egui::KeyboardShortcut::new(
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Key::C,
        );
        if ctx.input_mut(|input| input.consume_shortcut(&copy_table_shortcut)) {
            self.copy_visible_tsv(ctx);
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            self.request_kill_selected();
        }
    }
}

fn lines_from_vec(values: &[String]) -> String {
    values.join("\n")
}

fn classification_settings_equal(left: &Settings, right: &Settings) -> bool {
    left.protected_process_names == right.protected_process_names
        && left.python_keywords == right.python_keywords
        && left.codex_root_keywords == right.codex_root_keywords
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchField {
    Any,
    Pid,
    Name,
    Scope,
    Path,
    Command,
    Parent,
    Port,
    Web,
    State,
    Ram,
    Vram,
}

#[derive(Debug, Clone)]
struct SearchTerm {
    field: SearchField,
    value: String,
    integer: Option<u32>,
    numeric_mb: Option<NumericMbQuery>,
    excluded: bool,
}

#[derive(Debug, Clone, Copy)]
struct NumericMbQuery {
    operator: NumericMbOperator,
    megabytes: f64,
}

#[derive(Debug, Clone, Copy)]
enum NumericMbOperator {
    GreaterOrEqual,
    LessOrEqual,
    Greater,
    Less,
    Equal,
}

#[derive(Debug, Clone, Default)]
struct SearchQuery {
    terms: Vec<SearchTerm>,
}

impl SearchQuery {
    fn parse(query: &str) -> Self {
        let terms = split_search_tokens(query)
            .into_iter()
            .filter_map(|raw| {
                let (excluded, raw) = raw
                    .strip_prefix('-')
                    .map(|value| (true, value))
                    .unwrap_or((false, raw.as_str()));
                if raw.is_empty() {
                    return None;
                }
                let (field, value) = match raw.split_once(':') {
                    Some((field, value)) => match field.to_ascii_lowercase().as_str() {
                        "pid" => (SearchField::Pid, value),
                        "name" => (SearchField::Name, value),
                        "scope" => (SearchField::Scope, value),
                        "path" => (SearchField::Path, value),
                        "cmd" | "command" => (SearchField::Command, value),
                        "parent" => (SearchField::Parent, value),
                        "port" => (SearchField::Port, value),
                        "web" | "url" => (SearchField::Web, value),
                        "state" => (SearchField::State, value),
                        "ram" => (SearchField::Ram, value),
                        "vram" => (SearchField::Vram, value),
                        _ => (SearchField::Any, raw),
                    },
                    None => (SearchField::Any, raw),
                };
                (!value.is_empty()).then(|| {
                    let value = value.to_lowercase();
                    let integer = matches!(
                        field,
                        SearchField::Pid | SearchField::Parent | SearchField::Port
                    )
                    .then(|| parse_canonical_u32(&value))
                    .flatten();
                    let numeric_mb = matches!(field, SearchField::Ram | SearchField::Vram)
                        .then(|| NumericMbQuery::parse(&value))
                        .flatten();
                    SearchTerm {
                        field,
                        value,
                        integer,
                        numeric_mb,
                        excluded,
                    }
                })
            })
            .collect();
        Self { terms }
    }

    fn matches(&self, process: &ProcessInfo) -> bool {
        self.terms.iter().all(|term| {
            let matched = match term.field {
                SearchField::Any => process.searchable_text_lower.contains(&term.value),
                SearchField::Pid => term.integer == Some(process.pid),
                SearchField::Name => contains_lowercase(&process.name, &term.value),
                SearchField::Scope => contains_lowercase(process.scope.as_str(), &term.value),
                SearchField::Path => {
                    contains_lowercase(process.exe_path.as_deref().unwrap_or_default(), &term.value)
                }
                SearchField::Command => contains_lowercase(
                    process.command_line.as_deref().unwrap_or_default(),
                    &term.value,
                ),
                SearchField::Parent => {
                    contains_lowercase(
                        process.parent_name.as_deref().unwrap_or_default(),
                        &term.value,
                    ) || process
                        .parent_pid
                        .is_some_and(|pid| term.integer == Some(pid))
                }
                SearchField::Port => process
                    .local_endpoints
                    .iter()
                    .any(|endpoint| term.integer == Some(u32::from(endpoint.port))),
                SearchField::Web => process
                    .local_endpoints
                    .iter()
                    .any(|endpoint| contains_lowercase(&endpoint.url, &term.value)),
                SearchField::State => {
                    contains_lowercase(process.snapshot_state.as_str(), &term.value)
                }
                SearchField::Ram => term
                    .numeric_mb
                    .is_some_and(|query| query.matches(process.ram_bytes)),
                SearchField::Vram => process
                    .vram_bytes()
                    .is_some_and(|bytes| term.numeric_mb.is_some_and(|query| query.matches(bytes))),
            };
            matched != term.excluded
        })
    }
}

fn split_search_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    for character in query.chars() {
        match character {
            '"' => quoted = !quoted,
            character if character.is_whitespace() && !quoted => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            character => token.push(character),
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

impl NumericMbQuery {
    fn parse(expression: &str) -> Option<Self> {
        let (operator, value) = if let Some(value) = expression.strip_prefix(">=") {
            (NumericMbOperator::GreaterOrEqual, value)
        } else if let Some(value) = expression.strip_prefix("<=") {
            (NumericMbOperator::LessOrEqual, value)
        } else if let Some(value) = expression.strip_prefix('>') {
            (NumericMbOperator::Greater, value)
        } else if let Some(value) = expression.strip_prefix('<') {
            (NumericMbOperator::Less, value)
        } else if let Some(value) = expression.strip_prefix('=') {
            (NumericMbOperator::Equal, value)
        } else {
            (NumericMbOperator::GreaterOrEqual, expression)
        };
        let megabytes = value.parse::<f64>().ok()?;
        if !megabytes.is_finite() || megabytes < 0.0 {
            return None;
        }
        Some(Self {
            operator,
            megabytes,
        })
    }

    fn matches(self, bytes: u64) -> bool {
        let actual = bytes as f64 / 1024.0 / 1024.0;
        match self.operator {
            NumericMbOperator::GreaterOrEqual => actual >= self.megabytes,
            NumericMbOperator::LessOrEqual => actual <= self.megabytes,
            NumericMbOperator::Greater => actual > self.megabytes,
            NumericMbOperator::Less => actual < self.megabytes,
            NumericMbOperator::Equal => (actual - self.megabytes).abs() < 0.05,
        }
    }
}

fn parse_canonical_u32(value: &str) -> Option<u32> {
    let parsed = value.parse::<u32>().ok()?;
    (parsed.to_string() == value).then_some(parsed)
}

fn contains_lowercase(value: &str, lowercase_needle: &str) -> bool {
    if lowercase_needle.is_empty() {
        return true;
    }
    if value.is_ascii() && lowercase_needle.is_ascii() {
        let needle = lowercase_needle.as_bytes();
        return value
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    value.to_lowercase().contains(lowercase_needle)
}

fn sanitize_tsv_field(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

fn empty_as_unavailable(value: &str) -> &str {
    if value.is_empty() {
        "not loaded"
    } else {
        value
    }
}

fn vec_from_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

fn mb_to_bytes(value: u64) -> u64 {
    value.saturating_mul(1024 * 1024)
}

fn toggle_quick_filter(settings: &mut Settings, filter: QuickFilter) {
    match filter {
        QuickFilter::All => {
            settings.python_only = false;
            settings.gpu_active_only = false;
            settings.codex_related_only = false;
            settings.local_web_only = false;
            settings.heavy_ram_only = false;
            settings.heavy_vram_only = false;
            settings.memory_changed_only = false;
        }
        QuickFilter::Python => settings.python_only = !settings.python_only,
        QuickFilter::GpuActive => settings.gpu_active_only = !settings.gpu_active_only,
        QuickFilter::LocalWeb => settings.local_web_only = !settings.local_web_only,
        QuickFilter::CodexTerminal => {
            settings.codex_related_only = !settings.codex_related_only;
        }
        QuickFilter::HeavyRam => settings.heavy_ram_only = !settings.heavy_ram_only,
        QuickFilter::HeavyVram => settings.heavy_vram_only = !settings.heavy_vram_only,
        QuickFilter::Changed => settings.memory_changed_only = !settings.memory_changed_only,
    }
}

fn apply_snapshot_deltas(
    previous: &[ProcessInfo],
    current: &mut [ProcessInfo],
    has_baseline: bool,
) -> Option<SnapshotDeltaSummary> {
    if !has_baseline {
        return None;
    }

    let previous_by_pid = previous
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    let mut matched_previous = HashSet::new();
    let mut summary = SnapshotDeltaSummary::default();
    for process in current {
        let Some(previous) = previous_by_pid
            .get(&process.pid)
            .copied()
            .filter(|previous| process_identity::same_process_for_snapshot(previous, process))
        else {
            process.snapshot_state = SnapshotState::New;
            summary.started += 1;
            continue;
        };

        matched_previous.insert(previous.pid);
        process.ram_delta_bytes = Some(signed_byte_delta(process.ram_bytes, previous.ram_bytes));
        process.vram_delta_bytes = match (process.vram_bytes(), previous.vram_bytes()) {
            (Some(current), Some(previous)) => Some(signed_byte_delta(current, previous)),
            _ => None,
        };
        let changed = process.ram_delta_bytes != Some(0)
            || process.vram_delta_bytes.is_some_and(|delta| delta != 0);
        process.snapshot_state = if changed {
            summary.changed += 1;
            SnapshotState::Changed
        } else {
            SnapshotState::Unchanged
        };
    }
    summary.exited = previous
        .iter()
        .filter(|process| !matched_previous.contains(&process.pid))
        .count();
    Some(summary)
}

fn enrich_action_processes(current: &mut [ProcessInfo], loaded: &[ProcessInfo]) {
    let loaded_by_pid = loaded
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    for process in current {
        let Some(loaded) = loaded_by_pid
            .get(&process.pid)
            .copied()
            .filter(|loaded| process_identity::same_process(loaded, process))
        else {
            continue;
        };
        process.gpu = loaded.gpu.clone();
        process.local_endpoints = loaded.local_endpoints.clone();
        process.ram_delta_bytes = loaded.ram_delta_bytes;
        process.vram_delta_bytes = loaded.vram_delta_bytes;
        process.snapshot_state = loaded.snapshot_state;
        process.refresh_searchable_text();
    }
}

fn signed_byte_delta(current: u64, previous: u64) -> i64 {
    if current >= previous {
        current.saturating_sub(previous).min(i64::MAX as u64) as i64
    } else {
        -(previous.saturating_sub(current).min(i64::MAX as u64) as i64)
    }
}

fn explorer_select(path: &str) -> Command {
    let mut command = Command::new("explorer.exe");
    #[cfg(target_os = "windows")]
    {
        command.raw_arg(format!("/select,\"{}\"", path.replace('"', "")));
    }
    #[cfg(not(target_os = "windows"))]
    {
        command.arg(path);
    }
    command
}

fn process_summary(process: &ProcessInfo) -> String {
    let mut lines = vec![
        format!("Name: {}", process.name),
        format!("PID: {}", process.pid),
        format!(
            "RAM: {} MB",
            crate::services::formatter::bytes_to_mb_text(process.ram_bytes)
        ),
        format!(
            "VRAM: {} MB",
            crate::services::formatter::optional_bytes_to_mb_text(process.vram_bytes())
        ),
        format!("Scope: {}", process.scope),
        format!(
            "Age: {}",
            crate::services::formatter::age_text(process.start_time)
        ),
        format!("Snapshot State: {}", process.snapshot_state),
    ];
    if process.ram_delta_bytes.is_some() {
        lines.push(format!(
            "RAM Delta: {} MB",
            crate::services::formatter::optional_delta_mb_text(process.ram_delta_bytes)
        ));
    }
    if process.vram_delta_bytes.is_some() {
        lines.push(format!(
            "VRAM Delta: {} MB",
            crate::services::formatter::optional_delta_mb_text(process.vram_delta_bytes)
        ));
    }
    if let Some(parent_pid) = process.parent_pid {
        lines.push(format!(
            "Parent: {} ({parent_pid})",
            process.parent_name.as_deref().unwrap_or_default()
        ));
    }
    if let Some(path) = &process.exe_path {
        lines.push(format!("Path: {path}"));
    }
    if let Some(command_line) = &process.command_line {
        lines.push(format!("Command Line: {command_line}"));
    }
    let local_web = process.local_web_summary();
    if !local_web.is_empty() {
        lines.push(format!("Local Web: {local_web}"));
    }
    lines.join("\n")
}

fn tree_targets_from(processes: &[ProcessInfo], root_pid: u32) -> Vec<ProcessInfo> {
    let by_pid: HashMap<u32, &ProcessInfo> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    collect_tree(root_pid, &by_pid, &mut visited, &mut result);
    result
}

fn collect_tree(
    pid: u32,
    by_pid: &HashMap<u32, &ProcessInfo>,
    visited: &mut HashSet<u32>,
    result: &mut Vec<ProcessInfo>,
) {
    if !visited.insert(pid) {
        return;
    }
    let Some(process) = by_pid.get(&pid) else {
        return;
    };
    result.push((*process).clone());
    let mut children = process.children.clone();
    children.sort_unstable();
    for child_pid in children {
        collect_tree(child_pid, by_pid, visited, result);
    }
}

pub fn sort_processes(processes: &mut [ProcessInfo], sort: SortPreset) {
    match sort {
        SortPreset::RamAsc => processes.sort_by(|left, right| {
            left.ram_bytes
                .cmp(&right.ram_bytes)
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::RamDesc => processes.sort_by(|left, right| {
            right
                .ram_bytes
                .cmp(&left.ram_bytes)
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::VramAsc => processes.sort_by(|left, right| {
            compare_optional_bytes(left.vram_bytes(), right.vram_bytes(), false)
                .then_with(|| left.ram_bytes.cmp(&right.ram_bytes))
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::VramDesc => processes.sort_by(|left, right| {
            compare_optional_bytes(left.vram_bytes(), right.vram_bytes(), true)
                .then_with(|| right.ram_bytes.cmp(&left.ram_bytes))
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::NameAsc => processes.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.pid.cmp(&right.pid))
        }),
        SortPreset::NameDesc => processes.sort_by(|left, right| {
            right
                .name
                .cmp(&left.name)
                .then_with(|| left.pid.cmp(&right.pid))
        }),
        SortPreset::PidAsc => processes.sort_by_key(|process| process.pid),
        SortPreset::PidDesc => {
            processes.sort_by_key(|process| std::cmp::Reverse(process.pid));
        }
        SortPreset::AgeNewest => processes.sort_by(|left, right| {
            compare_optional_time(left.start_time, right.start_time, true)
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::AgeOldest => processes.sort_by(|left, right| {
            compare_optional_time(left.start_time, right.start_time, false)
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::RamGrowth => processes.sort_by(|left, right| {
            compare_optional_delta(left.ram_delta_bytes, right.ram_delta_bytes)
                .then_with(|| process_tie_breaker(left, right))
        }),
        SortPreset::VramGrowth => processes.sort_by(|left, right| {
            compare_optional_delta(left.vram_delta_bytes, right.vram_delta_bytes)
                .then_with(|| process_tie_breaker(left, right))
        }),
    }
}

fn process_tie_breaker(left: &ProcessInfo, right: &ProcessInfo) -> std::cmp::Ordering {
    left.name
        .cmp(&right.name)
        .then_with(|| left.pid.cmp(&right.pid))
}

fn compare_optional_bytes(
    left: Option<u64>,
    right: Option<u64>,
    descending: bool,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) if descending => right.cmp(&left),
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn compare_optional_delta(left: Option<i64>, right: Option<i64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn compare_optional_time(
    left: Option<SystemTime>,
    right: Option<SystemTime>,
    newest_first: bool,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) if newest_first => right.cmp(&left),
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ListeningEndpoint, ProcessScope};
    use std::time::{Duration, UNIX_EPOCH};

    fn process(pid: u32, name: &str, start_seconds: u64, ram_mb: u64) -> ProcessInfo {
        let mut process = ProcessInfo {
            pid,
            name: name.to_string(),
            exe_path: Some(format!(r"C:\Program Files\{name}")),
            start_time: Some(UNIX_EPOCH + Duration::from_secs(start_seconds)),
            ram_bytes: mb_to_bytes(ram_mb),
            ..Default::default()
        };
        process.refresh_searchable_text();
        process
    }

    #[test]
    fn filtered_out_selection_cannot_be_resolved_or_terminated() {
        let mut app = WatcherApp {
            processes: vec![ProcessInfo {
                pid: 42,
                name: "python.exe".to_string(),
                python_related: true,
                ..Default::default()
            }],
            selected_pid: Some(42),
            ..Default::default()
        };
        app.settings.gpu_active_only = true;

        assert!(app.selected_process().is_none());
        app.request_kill_selected();
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "No process selected.");
    }

    #[test]
    fn close_requires_confirmation() {
        let mut app = WatcherApp {
            processes: vec![ProcessInfo {
                pid: 42,
                name: "notepad.exe".to_string(),
                start_time: Some(SystemTime::now()),
                ..Default::default()
            }],
            selected_pid: Some(42),
            ..Default::default()
        };

        app.request_close_selected();

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::Close { ref targets }) if targets.len() == 1 && targets[0].pid == 42
        ));
    }

    #[test]
    fn completed_load_reclassifies_when_detection_settings_changed() {
        let loaded_settings = Settings {
            protected_process_names: Vec::new(),
            python_keywords: vec!["special-worker".to_string()],
            codex_root_keywords: Vec::new(),
            ..Default::default()
        };
        let mut current_settings = loaded_settings.clone();
        current_settings.python_keywords.clear();
        let mut loaded_process = process(999_999, "worker.exe", 100, 64);
        loaded_process.command_line = Some("special-worker".to_string());
        crate::services::scope_detector::assign_scopes(
            std::slice::from_mut(&mut loaded_process),
            &loaded_settings,
        );
        assert!(loaded_process.python_related);
        let snapshot = ProcessSnapshot {
            processes: vec![loaded_process],
            ..Default::default()
        };
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok((snapshot, loaded_settings))).unwrap();
        let mut app = WatcherApp {
            settings: current_settings,
            loading: true,
            load_receiver: Some(receiver),
            ..Default::default()
        };

        app.poll_load();

        assert!(!app.loading);
        assert!(!app.processes[0].python_related);
        assert!(app.processes[0]
            .searchable_text_lower
            .contains("worker.exe"));
    }

    #[test]
    fn structured_search_supports_fields_quotes_numeric_ranges_and_exclusions() {
        let mut process = process(42, "python.exe", 100, 2048);
        process.command_line = Some("python server.py --mode production".to_string());
        process.parent_name = Some("Code.exe".to_string());
        process.scope = ProcessScope::Python;
        process.snapshot_state = SnapshotState::Changed;
        process.local_endpoints =
            vec![ListeningEndpoint::new("127.0.0.1".to_string(), 7860, false)];
        process.refresh_searchable_text();

        assert!(SearchQuery::parse(
            r#"name:python path:"program files" parent:code port:7860 ram:>1024 -cmd:test state:changed"#
        )
        .matches(&process));
        assert!(!SearchQuery::parse("pid:4").matches(&process));
        assert!(!SearchQuery::parse("pid:042").matches(&process));
        assert!(SearchQuery::parse("name:PYTHON ram:>=2048").matches(&process));
        assert!(!SearchQuery::parse("vram:>0").matches(&process));
        assert!(!SearchQuery::parse("-cmd:production").matches(&process));
    }

    #[test]
    fn field_search_keeps_unicode_case_insensitive_matching() {
        let mut process = process(7, "ÄTool.exe", 100, 64);
        process.exe_path = Some(r"C:\ÄPP\ÄTool.exe".to_string());
        process.refresh_searchable_text();

        assert!(SearchQuery::parse("name:ätool path:äpp").matches(&process));
    }

    #[test]
    fn visible_view_collects_rows_and_stats_in_one_result() {
        let mut python = process(10, "python.exe", 100, 2048);
        python.python_related = true;
        python.local_endpoints = vec![ListeningEndpoint::new("127.0.0.1".to_string(), 7860, false)];
        python.refresh_searchable_text();
        let node = process(20, "node.exe", 100, 512);
        let app = WatcherApp {
            processes: vec![python, node],
            search: "name:python".to_string(),
            ..Default::default()
        };

        let visible = app.visible_process_view();

        assert_eq!(visible.indices, vec![0]);
        assert_eq!(visible.stats.rows, 1);
        assert_eq!(visible.stats.ram_bytes, mb_to_bytes(2048));
        assert_eq!(visible.stats.local_web_processes, 1);
    }

    #[test]
    fn quick_filters_toggle_independently_and_all_clears_them() {
        let mut settings = Settings::default();

        toggle_quick_filter(&mut settings, QuickFilter::Python);
        toggle_quick_filter(&mut settings, QuickFilter::LocalWeb);
        assert!(settings.python_only);
        assert!(settings.local_web_only);

        toggle_quick_filter(&mut settings, QuickFilter::Python);
        assert!(!settings.python_only);
        assert!(settings.local_web_only);

        toggle_quick_filter(&mut settings, QuickFilter::All);
        assert!(!settings.local_web_only);
        assert!(!settings.memory_changed_only);
    }

    #[test]
    fn snapshot_delta_tracks_started_exited_and_changed_by_identity() {
        let previous = vec![
            process(10, "python.exe", 100, 100),
            process(20, "node.exe", 100, 50),
        ];
        let mut current = vec![
            process(10, "python.exe", 100, 140),
            process(30, "cargo.exe", 100, 25),
        ];

        let summary = apply_snapshot_deltas(&previous, &mut current, true).unwrap();

        assert_eq!(summary.started, 1);
        assert_eq!(summary.exited, 1);
        assert_eq!(summary.changed, 1);
        assert_eq!(current[0].ram_delta_bytes, Some(mb_to_bytes(40) as i64));
        assert_eq!(current[0].snapshot_state, SnapshotState::Changed);
        assert_eq!(current[1].snapshot_state, SnapshotState::New);
    }

    #[test]
    fn snapshot_delta_treats_reused_pid_as_exit_and_start() {
        let previous = vec![process(10, "python.exe", 100, 100)];
        let mut current = vec![process(10, "python.exe", 101, 100)];

        let summary = apply_snapshot_deltas(&previous, &mut current, true).unwrap();

        assert_eq!(summary.started, 1);
        assert_eq!(summary.exited, 1);
        assert_eq!(summary.changed, 0);
        assert_eq!(current[0].ram_delta_bytes, None);
        assert_eq!(current[0].snapshot_state, SnapshotState::New);
    }

    #[test]
    fn snapshot_delta_does_not_report_uninspectable_stable_pids_as_restarted() {
        let mut previous = process(10, "System", 100, 100);
        previous.start_time = None;
        previous.exe_path = None;
        let mut current = vec![ProcessInfo {
            ram_bytes: mb_to_bytes(140),
            ..previous.clone()
        }];

        let summary = apply_snapshot_deltas(&[previous], &mut current, true).unwrap();

        assert_eq!(summary.started, 0);
        assert_eq!(summary.exited, 0);
        assert_eq!(summary.changed, 1);
        assert_eq!(current[0].ram_delta_bytes, Some(mb_to_bytes(40) as i64));
        assert_eq!(current[0].snapshot_state, SnapshotState::Changed);
    }

    #[test]
    fn growth_sort_places_known_largest_growth_first() {
        let mut processes = vec![
            ProcessInfo {
                pid: 1,
                name: "shrinking".to_string(),
                ram_delta_bytes: Some(-10),
                ..Default::default()
            },
            ProcessInfo {
                pid: 2,
                name: "unknown".to_string(),
                ram_delta_bytes: None,
                ..Default::default()
            },
            ProcessInfo {
                pid: 3,
                name: "growing".to_string(),
                ram_delta_bytes: Some(20),
                ..Default::default()
            },
        ];

        sort_processes(&mut processes, SortPreset::RamGrowth);

        assert_eq!(
            processes
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
    }
}
