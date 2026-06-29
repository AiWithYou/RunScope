use crate::app::WatcherApp;
use crate::model::SortPreset;
use crate::settings::{RefreshMode, TableView};

pub fn draw(ctx: &egui::Context, app: &mut WatcherApp) {
    if !app.show_settings {
        return;
    }

    let mut open = app.show_settings;
    egui::Window::new("Settings")
        .open(&mut open)
        .resizable(true)
        .default_width(680.0)
        .default_height(640.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_general(ui, app);
                    draw_filters(ui, app);
                    draw_protection(ui, app);
                    draw_keywords(ui, app);
                    draw_about(ui);

                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Save").clicked() {
                            app.save_settings_from_editor();
                        }
                        if ui.button("Reload settings.json").clicked() {
                            app.reload_settings_from_disk();
                        }
                        if ui.button("Open settings.json").clicked() {
                            app.open_settings_json();
                        }
                        if ui.button("Reset to default").clicked() {
                            app.reset_settings_to_default();
                        }
                    });
                });
        });
    app.show_settings = open;
}

fn draw_general(ui: &mut egui::Ui, app: &mut WatcherApp) {
    egui::CollapsingHeader::new("General")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Refresh mode");
                ui.selectable_value(
                    &mut app.settings.refresh_mode,
                    RefreshMode::Manual,
                    "Manual",
                );
                ui.selectable_value(&mut app.settings.refresh_mode, RefreshMode::Auto, "Auto");
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Refresh interval");
                egui::ComboBox::from_id_source("settings_refresh_interval")
                    .selected_text(format!("{}s", app.settings.auto_refresh_interval_ms / 1000))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut app.settings.auto_refresh_interval_ms, 2000, "2s");
                        ui.selectable_value(&mut app.settings.auto_refresh_interval_ms, 5000, "5s");
                        ui.selectable_value(
                            &mut app.settings.auto_refresh_interval_ms,
                            10000,
                            "10s",
                        );
                    });
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Default sort");
                let mut sort = app.sort;
                egui::ComboBox::from_id_source("settings_default_sort")
                    .selected_text(sort.as_settings_value())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sort, SortPreset::VramDesc, "VramDesc");
                        ui.selectable_value(&mut sort, SortPreset::RamDesc, "RamDesc");
                    });
                if sort != app.sort {
                    app.sort = sort;
                    app.settings.default_sort = sort.as_settings_value().to_string();
                    crate::app::sort_processes(&mut app.processes, app.sort);
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Table view");
                ui.selectable_value(&mut app.settings.table_view, TableView::Compact, "Compact");
                ui.selectable_value(
                    &mut app.settings.table_view,
                    TableView::Advanced,
                    "Advanced",
                );
            });
        });
}

fn draw_filters(ui: &mut egui::Ui, app: &mut WatcherApp) {
    egui::CollapsingHeader::new("Filters")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut app.settings.python_only, "Python only");
            ui.checkbox(&mut app.settings.gpu_active_only, "GPU/VRAM active only");
            ui.checkbox(&mut app.settings.local_web_only, "Local Web only");
            ui.checkbox(
                &mut app.settings.codex_related_only,
                "Codex/Terminal related only",
            );
            ui.checkbox(&mut app.settings.heavy_ram_only, "Heavy RAM");
            ui.horizontal_wrapped(|ui| {
                ui.label("Heavy RAM threshold MB");
                ui.add(
                    egui::DragValue::new(&mut app.settings.heavy_ram_threshold_mb)
                        .speed(128.0)
                        .range(0..=1_048_576),
                );
            });
            ui.checkbox(&mut app.settings.heavy_vram_only, "Heavy VRAM");
            ui.horizontal_wrapped(|ui| {
                ui.label("Heavy VRAM threshold MB");
                ui.add(
                    egui::DragValue::new(&mut app.settings.heavy_vram_threshold_mb)
                        .speed(128.0)
                        .range(0..=1_048_576),
                );
            });

            let mut hide_system = !app.settings.show_system_processes;
            if ui
                .checkbox(&mut hide_system, "Hide system/protected")
                .changed()
            {
                app.settings.show_system_processes = !hide_system;
            }
        });
}

fn draw_protection(ui: &mut egui::Ui, app: &mut WatcherApp) {
    egui::CollapsingHeader::new("Protection")
        .default_open(false)
        .show(ui, |ui| {
            ui.label("Protected process names");
            ui.add(
                egui::TextEdit::multiline(&mut app.protected_process_names_text)
                    .desired_rows(10)
                    .code_editor(),
            );
        });
}

fn draw_keywords(ui: &mut egui::Ui, app: &mut WatcherApp) {
    egui::CollapsingHeader::new("Keywords")
        .default_open(false)
        .show(ui, |ui| {
            ui.label("Python keywords");
            ui.add(
                egui::TextEdit::multiline(&mut app.python_keywords_text)
                    .desired_rows(8)
                    .code_editor(),
            );

            ui.label("Codex/Claude/Terminal root keywords");
            ui.add(
                egui::TextEdit::multiline(&mut app.codex_root_keywords_text)
                    .desired_rows(8)
                    .code_editor(),
            );
        });
}

fn draw_about(ui: &mut egui::Ui) {
    egui::CollapsingHeader::new("About")
        .default_open(false)
        .show(ui, |ui| {
            ui.monospace(format!("RunScope {}", env!("CARGO_PKG_VERSION")));
            ui.monospace("Rust / egui / eframe / sysinfo / windows crate");
            ui.monospace("Manual snapshot mode by default");
        });
}
