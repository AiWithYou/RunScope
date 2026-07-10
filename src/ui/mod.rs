use crate::app::{PendingAction, QuickFilter, WatcherApp};
use crate::model::SortPreset;
use crate::services::formatter;
use crate::settings::TableView;

pub mod detail_panel;
pub mod process_table;
pub mod settings_panel;

const DEFAULT_DETAIL_PANEL_HEIGHT: f32 = 240.0;
const MIN_DETAIL_PANEL_HEIGHT: f32 = 120.0;
const MAX_DETAIL_PANEL_HEIGHT: f32 = 520.0;
const MIN_PROCESS_TABLE_HEIGHT: f32 = 160.0;
const SPLITTER_HEIGHT: f32 = 8.0;

pub fn draw(ctx: &egui::Context, app: &mut WatcherApp) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.add_space(4.0);
        draw_toolbar_row(ui, ctx, app);
        ui.add_space(3.0);
        draw_filter_row(ui, app);
        ui.add_space(3.0);
        draw_sort_row(ui, app);
        ui.add_space(4.0);
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        draw_process_split(ui, ctx, app);
    });

    draw_pending_dialog(ctx, app);
    settings_panel::draw(ctx, app);
}

fn draw_process_split(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut WatcherApp) {
    let available_width = ui.available_width();
    let available_height = ui.available_height();
    if available_width <= 0.0 || available_height <= 0.0 {
        return;
    }
    let full_bottom = ui.cursor().min.y + available_height;

    if app.detail_panel_height <= 0.0 {
        app.detail_panel_height = DEFAULT_DETAIL_PANEL_HEIGHT;
    }
    app.detail_panel_height =
        clamped_detail_panel_height(app.detail_panel_height, available_height);
    let table_height = table_height_for(available_height, app.detail_panel_height);

    let (table_rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, table_height),
        egui::Sense::hover(),
    );
    let mut table_ui = ui.child_ui(table_rect, egui::Layout::top_down(egui::Align::LEFT), None);
    table_ui.set_clip_rect(table_rect);
    process_table::draw(&mut table_ui, app, table_rect.height());

    let (splitter_rect, resize_response) = ui.allocate_exact_size(
        egui::vec2(available_width, SPLITTER_HEIGHT),
        egui::Sense::drag(),
    );
    draw_splitter(ui, splitter_rect, &resize_response);

    if resize_response.hovered() || resize_response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resize_response.dragged() {
        if let Some(pointer) = resize_response.interact_pointer_pos() {
            let requested = full_bottom - pointer.y - SPLITTER_HEIGHT * 0.5;
            app.detail_panel_height = clamped_detail_panel_height(requested, available_height);
            ctx.request_repaint();
        }
    }

    let (detail_rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, app.detail_panel_height),
        egui::Sense::hover(),
    );
    let mut detail_ui = ui.child_ui(detail_rect, egui::Layout::top_down(egui::Align::LEFT), None);
    detail_ui.set_clip_rect(detail_rect);
    ui.painter()
        .rect_filled(detail_rect, 0.0, ui.visuals().extreme_bg_color);
    detail_ui.add_space(4.0);
    draw_status_bar(&mut detail_ui, app);
    detail_ui.separator();
    detail_panel::draw(&mut detail_ui, ctx, app);
}

fn clamped_detail_panel_height(requested: f32, full_height: f32) -> f32 {
    let usable_height = (full_height - SPLITTER_HEIGHT).max(0.0);
    if usable_height <= MIN_PROCESS_TABLE_HEIGHT {
        return (usable_height * 0.4).max(0.0);
    }

    let max_detail = (usable_height - MIN_PROCESS_TABLE_HEIGHT).min(MAX_DETAIL_PANEL_HEIGHT);
    let min_detail = MIN_DETAIL_PANEL_HEIGHT.min(max_detail);
    requested.clamp(min_detail, max_detail)
}

fn table_height_for(full_height: f32, detail_height: f32) -> f32 {
    (full_height - detail_height - SPLITTER_HEIGHT).max(0.0)
}

fn draw_splitter(ui: &mut egui::Ui, rect: egui::Rect, response: &egui::Response) {
    let visuals = ui.visuals();
    let fill = visuals.extreme_bg_color;
    let stroke = if response.hovered() || response.dragged() {
        visuals.widgets.hovered.fg_stroke
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, fill);
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.center().y),
            egui::pos2(rect.right(), rect.center().y),
        ],
        stroke,
    );
}

fn draw_toolbar_row(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut WatcherApp) {
    ui.horizontal_wrapped(|ui| {
        ui.heading("RunScope");
        ui.label("Lightweight RAM/VRAM Process Inspector");
        ui.separator();

        if ui
            .add_enabled(!app.loading, egui::Button::new("Load / Reload"))
            .clicked()
        {
            app.start_load(ctx);
        }

        let mut auto_refresh = app.settings.auto_refresh_enabled();
        if ui.checkbox(&mut auto_refresh, "Auto refresh").changed() {
            app.settings.set_auto_refresh_enabled(auto_refresh);
            app.save_settings_quietly();
        }

        ui.label("Refresh interval");
        let mut interval = app.settings.auto_refresh_interval_ms;
        egui::ComboBox::from_id_source("refresh_interval")
            .selected_text(format!("{}s", interval / 1000))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut interval, 2000, "2s");
                ui.selectable_value(&mut interval, 5000, "5s");
                ui.selectable_value(&mut interval, 10000, "10s");
            });
        if interval != app.settings.auto_refresh_interval_ms {
            app.settings.auto_refresh_interval_ms = interval;
            app.save_settings_quietly();
        }

        ui.separator();
        if ui.button("Settings").clicked() {
            app.open_settings_window();
        }
    });
}

fn draw_filter_row(ui: &mut egui::Ui, app: &mut WatcherApp) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Search");
        let search_id = egui::Id::new("runscope_search_text");
        let search_response = ui.add_sized(
            [320.0, 22.0],
            egui::TextEdit::singleline(&mut app.search).id_source(search_id),
        );
        if app.take_search_focus_request() {
            search_response.request_focus();
        }

        ui.separator();
        quick_filter_button(ui, app, QuickFilter::All, "All");
        quick_filter_button(ui, app, QuickFilter::Python, "Python");
        quick_filter_button(ui, app, QuickFilter::GpuActive, "GPU Active");
        quick_filter_button(ui, app, QuickFilter::LocalWeb, "Local Web");
        quick_filter_button(ui, app, QuickFilter::CodexTerminal, "Codex/Claude");
        quick_filter_button(ui, app, QuickFilter::HeavyRam, "Heavy RAM");
        quick_filter_button(ui, app, QuickFilter::HeavyVram, "Heavy VRAM");

        ui.separator();
        let mut hide_system = !app.settings.show_system_processes;
        if ui.checkbox(&mut hide_system, "Hide system").changed() {
            app.settings.show_system_processes = !hide_system;
            app.save_settings_quietly();
        }
    });
}

fn draw_sort_row(ui: &mut egui::Ui, app: &mut WatcherApp) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Sort");
        if ui
            .selectable_label(app.sort == SortPreset::VramDesc, "VRAM High")
            .clicked()
        {
            app.set_sort(SortPreset::VramDesc);
        }
        if ui
            .selectable_label(app.sort == SortPreset::RamDesc, "RAM High")
            .clicked()
        {
            app.set_sort(SortPreset::RamDesc);
        }

        ui.separator();
        ui.label("View");
        if ui
            .selectable_label(app.settings.table_view == TableView::Compact, "Compact")
            .clicked()
        {
            app.set_table_view(TableView::Compact);
        }
        if ui
            .selectable_label(app.settings.table_view == TableView::Advanced, "Advanced")
            .clicked()
        {
            app.set_table_view(TableView::Advanced);
        }

        ui.separator();
        ui.label(format!(
            "Rows: {} / Total: {}",
            app.visible_process_count(),
            app.processes.len()
        ));
        if let Some(pid) = app.selected_pid {
            ui.separator();
            ui.label(format!("Selected PID: {pid}"));
        }
    });
}

fn quick_filter_button(ui: &mut egui::Ui, app: &mut WatcherApp, filter: QuickFilter, label: &str) {
    if ui
        .selectable_label(app.active_quick_filter() == filter, label)
        .clicked()
    {
        app.apply_quick_filter(filter);
    }
}

fn draw_status_bar(ui: &mut egui::Ui, app: &WatcherApp) {
    ui.horizontal_wrapped(|ui| {
        let last_updated = app
            .last_updated
            .map(last_updated_text)
            .unwrap_or_else(|| "Last updated: never".to_string());
        ui.label(last_updated);
        ui.separator();
        if app.loading {
            ui.spinner();
        }
        ui.label(&app.status);
        if !app.vram_status.is_empty() {
            ui.separator();
            ui.label(&app.vram_status);
        }
        if !app.listener_status.is_empty() {
            ui.separator();
            ui.label(&app.listener_status);
        }
    });
}

fn draw_pending_dialog(ctx: &egui::Context, app: &mut WatcherApp) {
    let Some(action) = app.pending_action.clone() else {
        return;
    };

    let (title, confirm_label, description, targets) = match &action {
        PendingAction::Close { targets } => (
            "Close process",
            "Close",
            "The selected process will be asked to close.",
            targets,
        ),
        PendingAction::Kill { targets } => (
            "Kill process",
            "Kill",
            "The selected process will be terminated.",
            targets,
        ),
        PendingAction::KillTree { targets } => (
            "Kill process tree",
            "Kill Tree",
            "These processes will be terminated.",
            targets,
        ),
    };

    let screen_rect = ctx.screen_rect();
    egui::Area::new(egui::Id::new("pending_action_backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen_rect.min)
        .show(ctx, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(screen_rect.size(), egui::Sense::click_and_drag());
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(96));
        });

    egui::Window::new(title)
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(true)
        .default_width(620.0)
        .show(ctx, |ui| {
            ui.label(description);
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for process in targets {
                        let local_web = process.local_web_summary();
                        ui.monospace(format!(
                            "PID {:>6}  {:<28} RAM {:>8} MB  VRAM {:>8} MB  {}",
                            process.pid,
                            process.name,
                            formatter::bytes_to_mb_text(process.ram_bytes),
                            formatter::optional_bytes_to_mb_text(process.vram_bytes()),
                            local_web
                        ));
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    app.cancel_pending_action();
                }
                if ui
                    .add(
                        egui::Button::new(confirm_label).fill(egui::Color32::from_rgb(145, 40, 40)),
                    )
                    .clicked()
                {
                    app.confirm_pending_action(ctx);
                }
            });
        });
}

fn last_updated_text(time: std::time::SystemTime) -> String {
    match std::time::SystemTime::now().duration_since(time) {
        Ok(elapsed) if elapsed.as_secs() < 2 => "Last updated: just now".to_string(),
        Ok(elapsed) => format!("Last updated: {}s ago", elapsed.as_secs()),
        Err(_) => "Last updated: just now".to_string(),
    }
}
