use crate::app::WatcherApp;
use crate::model::ProcessInfo;
use crate::services::formatter;
use std::collections::HashSet;

pub fn draw(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut WatcherApp) {
    if app.settings.auto_refresh_enabled() {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    let Some(process) = app.selected_process().cloned() else {
        ui.label("Select a process row to inspect details and use process actions.");
        return;
    };

    egui::ScrollArea::vertical()
        .id_source("detail_panel_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_action_card(ui, ctx, app, &process);
            ui.add_space(6.0);
            draw_summary_card(ui, &process);
            ui.add_space(6.0);
            draw_local_web_card(ui, &process);
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
            ui.separator();
            ui.strong("Terminate");
            if ui
                .add_enabled(!process.protected, egui::Button::new("Close"))
                .clicked()
            {
                app.close_selected();
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
            if ui.button("Copy Summary").clicked() {
                app.copy_selected_summary(ctx);
            }
        });
    });
}

fn draw_local_web_card(ui: &mut egui::Ui, process: &ProcessInfo) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("Local Web");
        if process.local_endpoints.is_empty() {
            ui.label("No local listening ports found for this PID.");
            return;
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
                "GPU         : device {} {} {:?}",
                gpu.device_index, gpu.device_name, gpu.process_type
            ));
        }
    });
}

fn draw_children_card(ui: &mut egui::Ui, app: &WatcherApp, process: &ProcessInfo) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("Child processes");
        if process.children.is_empty() {
            ui.label("No child processes in the loaded snapshot.");
            return;
        }

        let mut lines = Vec::new();
        let mut visited = HashSet::new();
        collect_child_lines(&app.processes, process.pid, 0, &mut visited, &mut lines);
        for line in lines.iter().take(60) {
            ui.monospace(line);
        }
        if lines.len() > 60 {
            ui.label(format!("{} more child rows omitted.", lines.len() - 60));
        }
    });
}

fn collect_child_lines(
    processes: &[ProcessInfo],
    pid: u32,
    depth: usize,
    visited: &mut HashSet<u32>,
    lines: &mut Vec<String>,
) {
    if !visited.insert(pid) {
        return;
    }
    let Some(process) = processes.iter().find(|process| process.pid == pid) else {
        return;
    };
    let mut children = process.children.clone();
    children.sort_unstable();
    for child_pid in children {
        if let Some(child) = processes.iter().find(|process| process.pid == child_pid) {
            lines.push(format!(
                "{}PID {:>6}  {:<28} RAM {:>8} MB  VRAM {:>8} MB",
                "  ".repeat(depth),
                child.pid,
                child.name,
                formatter::bytes_to_mb_text(child.ram_bytes),
                formatter::optional_bytes_to_mb_text(child.vram_bytes())
            ));
        }
        collect_child_lines(processes, child_pid, depth + 1, visited, lines);
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.separator();
    ui.label(label);
    ui.monospace(value);
}
