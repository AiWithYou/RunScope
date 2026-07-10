use crate::collectors::process_collector;
use crate::model::{ProcessInfo, ProcessSnapshot, SortPreset};
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
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub struct WatcherApp {
    pub settings: Settings,
    pub settings_path: PathBuf,
    pub processes: Vec<ProcessInfo>,
    pub sort: SortPreset,
    pub search: String,
    pub status: String,
    pub vram_status: String,
    pub listener_status: String,
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
    load_receiver: Option<Receiver<anyhow::Result<ProcessSnapshot>>>,
    last_auto_refresh: Instant,
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
            load_receiver: None,
            last_auto_refresh: Instant::now(),
        }
    }
}

impl eframe::App for WatcherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keyboard_shortcuts(ctx);
        self.poll_load();
        self.maybe_auto_refresh(ctx);
        ui::draw(ctx, self);
    }
}

impl WatcherApp {
    pub fn start_load(&mut self, ctx: &egui::Context) {
        if self.loading {
            return;
        }

        self.loading = true;
        self.status = "Loading process snapshot...".to_string();
        self.last_auto_refresh = Instant::now();
        let settings = self.settings.clone();
        let (sender, receiver) = mpsc::channel();
        self.load_receiver = Some(receiver);
        let repaint = ctx.clone();

        thread::spawn(move || {
            let result = process_collector::collect_processes(&settings);
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    }

    pub fn set_sort(&mut self, sort: SortPreset) {
        self.sort = sort;
        self.settings.default_sort = sort.as_settings_value().to_string();
        sort_processes(&mut self.processes, self.sort);
        self.save_settings_quietly();
    }

    pub fn set_table_view(&mut self, table_view: TableView) {
        self.settings.table_view = table_view;
        self.save_settings_quietly();
    }

    pub fn visible_process_indices(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        self.processes
            .iter()
            .enumerate()
            .filter_map(|(index, process)| {
                self.process_matches_filters(process, &query)
                    .then_some(index)
            })
            .collect()
    }

    pub fn visible_process_count(&self) -> usize {
        let query = self.search.trim().to_lowercase();
        self.processes
            .iter()
            .filter(|process| self.process_matches_filters(process, &query))
            .count()
    }

    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        let query = self.search.trim().to_lowercase();
        self.selected_pid.and_then(|pid| {
            self.processes
                .iter()
                .find(|process| process.pid == pid)
                .filter(|process| self.process_matches_filters(process, &query))
        })
    }

    pub fn selected_process_summary(&self) -> Option<String> {
        self.selected_process().map(process_summary)
    }

    pub fn take_search_focus_request(&mut self) -> bool {
        let requested = self.focus_search_requested;
        self.focus_search_requested = false;
        requested
    }

    pub fn apply_quick_filter(&mut self, filter: QuickFilter) {
        self.settings.python_only = filter == QuickFilter::Python;
        self.settings.gpu_active_only = filter == QuickFilter::GpuActive;
        self.settings.codex_related_only = filter == QuickFilter::CodexTerminal;
        self.settings.local_web_only = filter == QuickFilter::LocalWeb;
        self.settings.heavy_ram_only = filter == QuickFilter::HeavyRam;
        self.settings.heavy_vram_only = filter == QuickFilter::HeavyVram;
        self.save_settings_quietly();
    }

    pub fn active_quick_filter(&self) -> QuickFilter {
        if self.settings.python_only {
            QuickFilter::Python
        } else if self.settings.gpu_active_only {
            QuickFilter::GpuActive
        } else if self.settings.local_web_only {
            QuickFilter::LocalWeb
        } else if self.settings.codex_related_only {
            QuickFilter::CodexTerminal
        } else if self.settings.heavy_ram_only {
            QuickFilter::HeavyRam
        } else if self.settings.heavy_vram_only {
            QuickFilter::HeavyVram
        } else {
            QuickFilter::All
        }
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
        let current_processes =
            match process_collector::collect_processes_for_action(&self.settings) {
                Ok(processes) => processes,
                Err(error) => {
                    self.status = format!("Kill Tree pre-check failed: {error}");
                    return;
                }
            };
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
                            "WM_CLOSE sent to {count} window(s) for PID {}.",
                            process.pid
                        );
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
                        self.status = format!("Killed PID {}.", process.pid);
                        self.start_load(ctx);
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
                let current_processes =
                    match process_collector::collect_processes_for_action(&self.settings) {
                        Ok(processes) => processes,
                        Err(error) => {
                            self.status = format!("Kill Tree pre-check failed: {error}");
                            return;
                        }
                    };
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
                            "Killed process tree rooted at PID {root_pid}: {:?}.",
                            killed
                        );
                        self.start_load(ctx);
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
        self.show_settings = true;
    }

    pub fn save_settings_from_editor(&mut self) {
        self.settings.protected_process_names = vec_from_lines(&self.protected_process_names_text);
        self.settings.python_keywords = vec_from_lines(&self.python_keywords_text);
        self.settings.codex_root_keywords = vec_from_lines(&self.codex_root_keywords_text);
        self.settings.default_sort = self.sort.as_settings_value().to_string();
        if self.save_settings_quietly() {
            self.status = "Settings saved.".to_string();
        }
    }

    pub fn reload_settings_from_disk(&mut self) {
        match Settings::load_or_default(&self.settings_path) {
            Ok(settings) => {
                self.settings = settings;
                self.sort = SortPreset::from_settings(&self.settings.default_sort);
                sort_processes(&mut self.processes, self.sort);
                self.sync_settings_editor();
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
        sort_processes(&mut self.processes, self.sort);
        self.sync_settings_editor();
        if self.save_settings_quietly() {
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
        match result {
            Ok(snapshot) => {
                self.processes = snapshot.processes;
                sort_processes(&mut self.processes, self.sort);
                self.last_updated = Some(SystemTime::now());
                self.vram_status = snapshot.vram_status;
                self.listener_status = snapshot.listener_status;
                self.status = format!("Loaded {} processes.", self.processes.len());
                if let Some(pid) = self.selected_pid {
                    if !self.processes.iter().any(|process| process.pid == pid) {
                        self.selected_pid = None;
                    }
                }
            }
            Err(error) => {
                self.status = format!("Load failed: {error}");
            }
        }
    }

    fn maybe_auto_refresh(&mut self, ctx: &egui::Context) {
        if !self.settings.auto_refresh_enabled() || self.loading {
            return;
        }
        let interval = Duration::from_millis(self.settings.auto_refresh_interval_ms.max(1000));
        if self.last_auto_refresh.elapsed() >= interval {
            self.start_load(ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn process_matches_filters(&self, process: &ProcessInfo, query: &str) -> bool {
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
        if !query.is_empty() && !process.searchable_text_lower.contains(query) {
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

        let focus_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::F);
        if ctx.input_mut(|input| input.consume_shortcut(&focus_shortcut)) {
            self.focus_search_requested = true;
        }

        if ctx.wants_keyboard_input() {
            return;
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            self.open_selected_local_web(ctx);
        }

        let copy_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::C);
        if ctx.input_mut(|input| input.consume_shortcut(&copy_shortcut)) {
            self.copy_selected_summary(ctx);
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            self.request_kill_selected();
        }
    }
}

fn lines_from_vec(values: &[String]) -> String {
    values.join("\n")
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
    ];
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
        SortPreset::RamDesc => processes.sort_by(|left, right| {
            right
                .ram_bytes
                .cmp(&left.ram_bytes)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.pid.cmp(&right.pid))
        }),
        SortPreset::VramDesc => {
            processes.sort_by(
                |left, right| match (left.vram_bytes(), right.vram_bytes()) {
                    (Some(left_vram), Some(right_vram)) => right_vram
                        .cmp(&left_vram)
                        .then_with(|| right.ram_bytes.cmp(&left.ram_bytes))
                        .then_with(|| left.name.cmp(&right.name))
                        .then_with(|| left.pid.cmp(&right.pid)),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => right
                        .ram_bytes
                        .cmp(&left.ram_bytes)
                        .then_with(|| left.name.cmp(&right.name))
                        .then_with(|| left.pid.cmp(&right.pid)),
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
