use crate::app::WatcherApp;
use crate::model::ProcessInfo;
use crate::services::formatter;
use std::collections::{HashMap, HashSet};

pub fn draw(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut WatcherApp, rows: &[usize]) {
    let visible_selection = app.selected_pid.and_then(|pid| {
        rows.iter()
            .map(|index| &app.processes[*index])
            .find(|process| process.pid == pid)
    });
    let Some(process) = visible_selection
        .or_else(|| app.selected_process())
        .cloned()
    else {
        ui.label("Select a process row to inspect details and use process actions.");
        return;
    };

    egui::ScrollArea::vertical()
        .id_source((
            "detail_panel_scroll",
            process.pid,
            process.start_time.and_then(|time| {
                time.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|duration| duration.as_secs())
            }),
        ))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_action_card(ui, ctx, app, &process);
            ui.add_space(6.0);
            draw_summary_card(ui, &process);
            ui.add_space(6.0);
            draw_local_web_card(ui, ctx, app, &process);
            ui.add_space(6.0);
            draw_details_card(ui, &process);
            ui.add_space(6.0);
            draw_children_card(ui, app, &process);
        });
}

fn draw_summary_card(ui: &mut egui::Ui, process: &ProcessInfo) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong(&process.name);
            metric(ui, "PID", process.pid.to_string());
            metric(
                ui,
                "RAM",
                format!("{} MB", formatter::bytes_to_mb_text(process.ram_bytes)),
            );
            metric(
                ui,
                "VRAM",
                format!(
                    "{} MB",
                    formatter::optional_bytes_to_mb_text(process.vram_bytes())
                ),
            );
            metric(ui, "Age", formatter::age_text(process.start_time));
            metric(ui, "State", process.snapshot_state.to_string());
            if process.ram_delta_bytes.is_some() {
                metric(
                    ui,
                    "RAM delta",
                    format!(
                        "{} MB",
                        formatter::optional_delta_mb_text(process.ram_delta_bytes)
                    ),
                );
            }
            if process.vram_delta_bytes.is_some() {
                metric(
                    ui,
                    "VRAM delta",
                    format!(
                        "{} MB",
                        formatter::optional_delta_mb_text(process.vram_delta_bytes)
                    ),
                );
            }
            metric(
                ui,
                "Parent",
                format!(
                    "{} ({})",
                    process.parent_name.as_deref().unwrap_or_default(),
                    process
                        .parent_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_default()
                ),
            );
            metric(ui, "Scope", process.scope.to_string());
            if process.protected {
                metric(
                    ui,
                    "Protected",
                    process
                        .protection_reason
                        .clone()
                        .unwrap_or_else(|| "true".to_string()),
                );
            }
        });
    });
}

fn draw_action_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut WatcherApp,
    process: &ProcessInfo,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Open");
            if ui
                .add_enabled(
                    process.primary_local_web_url().is_some(),
                    egui::Button::new("Open Local Web"),
                )
                .clicked()
            {
                app.open_selected_local_web(ctx);
            }
            if ui
                .add_enabled(
                    process.exe_path.is_some(),
                    egui::Button::new("Show EXE in Explorer"),
                )
                .on_hover_text("Select the executable in Windows Explorer.")
                .clicked()
            {
                app.reveal_selected_executable();
            }
            if ui
                .add_enabled(
                    process.exe_path.is_some(),
                    egui::Button::new("Open EXE Folder"),
                )
                .on_hover_text("Open the executable folder in Windows Explorer.")
                .clicked()
            {
                app.open_selected_executable_folder();
            }
            if ui
                .add_enabled(process.cwd.is_some(), egui::Button::new("Open CWD"))
                .on_hover_text("Open the process working directory in Windows Explorer.")
                .clicked()
            {
                app.open_selected_working_directory();
            }
            if let Some(parent_pid) = process.parent_pid {
                let parent_available = app
                    .processes
                    .iter()
                    .any(|candidate| candidate.pid == parent_pid);
                if ui
                    .add_enabled(parent_available, egui::Button::new("Select Parent"))
                    .clicked()
                {
                    app.select_pid(parent_pid);
                }
            }
            ui.separator();
            ui.strong("Terminate");
            if ui
                .add_enabled(!process.protected, egui::Button::new("Close"))
                .clicked()
            {
                app.request_close_selected();
            }
            if ui
                .add_enabled(!process.protected, egui::Button::new("Kill"))
                .clicked()
            {
                app.request_kill_selected();
            }
            if ui
                .add_enabled(!process.protected, egui::Button::new("Kill Tree"))
                .clicked()
            {
                app.request_kill_tree_selected();
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.strong("Copy");
            if ui.button("Copy PID").clicked() {
                ctx.copy_text(process.pid.to_string());
                app.status = "Copied PID.".to_string();
            }
            if ui.button("Copy Name").clicked() {
                ctx.copy_text(process.name.clone());
                app.status = "Copied process name.".to_string();
            }
            if ui
                .add_enabled(process.exe_path.is_some(), egui::Button::new("Copy Path"))
                .clicked()
            {
                ctx.copy_text(process.exe_path.clone().unwrap_or_default());
                app.status = "Copied executable path.".to_string();
            }
            if ui
                .add_enabled(
                    process.command_line.is_some(),
                    egui::Button::new("Copy Command"),
                )
                .clicked()
            {
                ctx.copy_text(process.command_line.clone().unwrap_or_default());
                app.status = "Copied command line.".to_string();
            }
            if ui
                .add_enabled(process.cwd.is_some(), egui::Button::new("Copy CWD"))
                .clicked()
            {
                ctx.copy_text(process.cwd.clone().unwrap_or_default());
                app.status = "Copied working directory.".to_string();
            }
            if ui.button("Copy Summary").clicked() {
                app.copy_selected_summary(ctx);
            }
            if ui.button("Copy JSON").clicked() {
                app.copy_selected_json(ctx);
            }
        });
    });
}

fn draw_local_web_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut WatcherApp,
    process: &ProcessInfo,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("Local Web");
        if process.local_endpoints.is_empty() {
            ui.label("No local listening ports found for this PID.");
            return;
        }
        if ui.button("Copy all URLs").clicked() {
            ctx.copy_text(process.local_web_summary());
            app.status = "Copied Local Web URLs.".to_string();
        }
        for endpoint in &process.local_endpoints {
            ui.horizontal(|ui| {
                ui.monospace(format!("{}:{} ->", endpoint.bind_address, endpoint.port));
                ui.hyperlink_to(&endpoint.url, &endpoint.url);
            });
        }
    });
}

fn draw_details_card(ui: &mut egui::Ui, process: &ProcessInfo) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("Details");
        ui.monospace(format!(
            "Path        : {}",
            process.exe_path.as_deref().unwrap_or_default()
        ));
        ui.monospace(format!(
            "CommandLine : {}",
            process.command_line.as_deref().unwrap_or_default()
        ));
        ui.monospace(format!(
            "CWD         : {}",
            process.cwd.as_deref().unwrap_or_default()
        ));
        ui.monospace(format!(
            "Virtual Mem : {} MB",
            formatter::bytes_to_mb_text(process.virtual_memory_bytes)
        ));
        ui.monospace(format!("Python      : {}", process.python_related));
        ui.monospace(format!("Codex/Claude: {}", process.codex_related));
        if let Some(gpu) = &process.gpu {
            ui.monospace(format!(
                "GPU         : devices [{}] {} {:?}",
                if gpu.device_indices.is_empty() {
                    "unknown".to_string()
                } else {
                    gpu.device_indices
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                gpu.device_names.join(", "),
                gpu.process_type
            ));
        }
    });
}

fn draw_children_card(ui: &mut egui::Ui, app: &mut WatcherApp, process: &ProcessInfo) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("Child processes");
        if process.children.is_empty() {
            ui.label("No child processes in the loaded snapshot.");
            return;
        }

        let by_pid = app
            .processes
            .iter()
            .map(|process| (process.pid, process))
            .collect::<HashMap<_, _>>();
        let mut rows = Vec::new();
        let mut visited = HashSet::new();
        collect_child_rows(&by_pid, process.pid, 0, &mut visited, &mut rows);
        for row in rows.iter().take(60) {
            ui.horizontal(|ui| {
                ui.add_space(row.depth as f32 * 16.0);
                if ui
                    .selectable_label(
                        false,
                        format!(
                            "PID {:>6}  {:<28} RAM {:>8} MB  VRAM {:>8} MB",
                            row.pid, row.name, row.ram_mb, row.vram_mb
                        ),
                    )
                    .clicked()
                {
                    app.select_pid(row.pid);
                    app.status = format!("Selected child PID {}.", row.pid);
                }
            });
        }
        if rows.len() > 60 {
            ui.label(format!("{} more child rows omitted.", rows.len() - 60));
        }
    });
}

struct ChildRow {
    depth: usize,
    pid: u32,
    name: String,
    ram_mb: String,
    vram_mb: String,
}

fn collect_child_rows(
    by_pid: &HashMap<u32, &ProcessInfo>,
    pid: u32,
    depth: usize,
    visited: &mut HashSet<u32>,
    rows: &mut Vec<ChildRow>,
) {
    if !visited.insert(pid) {
        return;
    }
    let Some(process) = by_pid.get(&pid) else {
        return;
    };
    let mut children = process.children.clone();
    children.sort_unstable();
    for child_pid in children {
        if let Some(child) = by_pid.get(&child_pid) {
            rows.push(ChildRow {
                depth,
                pid: child.pid,
                name: child.name.clone(),
                ram_mb: formatter::bytes_to_mb_text(child.ram_bytes),
                vram_mb: formatter::optional_bytes_to_mb_text(child.vram_bytes()),
            });
        }
        collect_child_rows(by_pid, child_pid, depth + 1, visited, rows);
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.separator();
    ui.label(label);
    ui.monospace(value);
}
