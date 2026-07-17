use crate::app::WatcherApp;
use crate::model::{ProcessInfo, ProcessScope, SortPreset};
use crate::services::formatter;
use crate::settings::TableView;

const ROW_HEIGHT: f32 = 30.0;
const CELL_PAD_X: f32 = 10.0;
const SCOPE_WIDTH: f32 = 104.0;
const PID_WIDTH: f32 = 86.0;
const NAME_WIDTH: f32 = 280.0;
const RAM_WIDTH: f32 = 134.0;
const VRAM_WIDTH: f32 = 134.0;
const LOCAL_WEB_WIDTH: f32 = 300.0;
const AGE_WIDTH: f32 = 104.0;
const PARENT_PID_WIDTH: f32 = 94.0;
const PARENT_NAME_WIDTH: f32 = 190.0;
const PATH_WIDTH: f32 = 380.0;
const COMMAND_WIDTH: f32 = 520.0;

#[derive(Clone, Copy)]
enum CellAlign {
    Left,
    Right,
}

enum TableAction {
    Close(u32),
    Kill(u32),
    KillTree(u32),
}

pub fn draw(ui: &mut egui::Ui, app: &mut WatcherApp, max_height: f32, rows: &[usize]) {
    let advanced = app.settings.table_view == TableView::Advanced;
    let mut selected_pid = app
        .selected_pid
        .filter(|pid| rows.iter().any(|index| app.processes[*index].pid == *pid));
    let mut table_action = None;
    let mut requested_sort = None;
    let table_height = max_height.max(0.0);
    let scroll_request = app.take_table_scroll_request();

    let mut scroll_area = egui::ScrollArea::both()
        .id_source("process_table_scroll")
        .auto_shrink([false, false])
        .max_height(table_height);
    if let Some(row) = scroll_request {
        scroll_area = scroll_area.vertical_scroll_offset((row + 1) as f32 * ROW_HEIGHT);
    }
    scroll_area.show_rows(ui, ROW_HEIGHT, rows.len() + 1, |ui, visible_rows| {
        ui.set_min_width(table_width(advanced));
        for visible_row in visible_rows {
            if visible_row == 0 {
                requested_sort = draw_header(ui, advanced, app.sort).or(requested_sort);
                if rows.is_empty() {
                    if app.loading {
                        ui.label("Loading process snapshot...");
                    } else if app.last_updated.is_none() {
                        ui.label(
                            "Click Load / Reload (or press F5) to collect the first snapshot.",
                        );
                    } else {
                        ui.label("No processes match the current search and filters.");
                    }
                }
                continue;
            }

            let row = visible_row - 1;
            let process = &app.processes[rows[row]];
            let selected = selected_pid == Some(process.pid);
            draw_process_row(
                ui,
                process,
                row,
                selected,
                advanced,
                &mut selected_pid,
                &mut table_action,
            );
        }
    });

    app.selected_pid = selected_pid;
    if let Some(sort) = requested_sort {
        app.set_sort(sort);
    }
    if let Some(action) = table_action {
        match action {
            TableAction::Close(pid) => {
                app.selected_pid = Some(pid);
                app.request_close_selected();
            }
            TableAction::Kill(pid) => {
                app.selected_pid = Some(pid);
                app.request_kill_selected();
            }
            TableAction::KillTree(pid) => {
                app.selected_pid = Some(pid);
                app.request_kill_tree_selected();
            }
        }
    }
}

fn draw_header(ui: &mut egui::Ui, advanced: bool, current_sort: SortPreset) -> Option<SortPreset> {
    let width = table_width(advanced).max(ui.available_width());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);

    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().strong_text_color();
    let mut x = rect.left();
    paint_header_cell(
        ui,
        next_rect(rect, &mut x, SCOPE_WIDTH),
        "Scope",
        CellAlign::Left,
    );
    paint_header_cell(
        ui,
        next_rect(rect, &mut x, PID_WIDTH),
        "PID",
        CellAlign::Right,
    );
    paint_header_cell(
        ui,
        next_rect(rect, &mut x, NAME_WIDTH),
        "Process Name",
        CellAlign::Left,
    );
    paint_text(
        ui,
        next_rect(rect, &mut x, RAM_WIDTH),
        "RAM MB (delta)",
        CellAlign::Right,
        color,
        font.clone(),
    );
    paint_text(
        ui,
        next_rect(rect, &mut x, VRAM_WIDTH),
        "VRAM MB (delta)",
        CellAlign::Right,
        color,
        font.clone(),
    );
    paint_header_cell(
        ui,
        next_rect(rect, &mut x, LOCAL_WEB_WIDTH),
        "Local Web",
        CellAlign::Left,
    );
    paint_header_cell(
        ui,
        next_rect(rect, &mut x, AGE_WIDTH),
        "Age",
        CellAlign::Left,
    );
    if advanced {
        paint_header_cell(
            ui,
            next_rect(rect, &mut x, PARENT_PID_WIDTH),
            "Parent PID",
            CellAlign::Right,
        );
        paint_header_cell(
            ui,
            next_rect(rect, &mut x, PARENT_NAME_WIDTH),
            "Parent Name",
            CellAlign::Left,
        );
        paint_header_cell(
            ui,
            next_rect(rect, &mut x, PATH_WIDTH),
            "Executable Path",
            CellAlign::Left,
        );
        paint_header_cell(
            ui,
            next_rect(rect, &mut x, COMMAND_WIDTH),
            "Command Line",
            CellAlign::Left,
        );
    }

    let response =
        response.on_hover_text("Click PID, Process Name, RAM, VRAM, or Age to toggle sorting.");
    if response.clicked() {
        response
            .interact_pointer_pos()
            .and_then(|position| sort_for_header_x(position.x - rect.left(), current_sort))
    } else {
        None
    }
}

fn sort_for_header_x(x: f32, current: SortPreset) -> Option<SortPreset> {
    let mut edge = SCOPE_WIDTH;
    if x < edge {
        return None;
    }
    edge += PID_WIDTH;
    if x < edge {
        return Some(if current == SortPreset::PidAsc {
            SortPreset::PidDesc
        } else {
            SortPreset::PidAsc
        });
    }
    edge += NAME_WIDTH;
    if x < edge {
        return Some(if current == SortPreset::NameAsc {
            SortPreset::NameDesc
        } else {
            SortPreset::NameAsc
        });
    }
    edge += RAM_WIDTH;
    if x < edge {
        return Some(if current == SortPreset::RamDesc {
            SortPreset::RamAsc
        } else {
            SortPreset::RamDesc
        });
    }
    edge += VRAM_WIDTH;
    if x < edge {
        return Some(if current == SortPreset::VramDesc {
            SortPreset::VramAsc
        } else {
            SortPreset::VramDesc
        });
    }
    edge += LOCAL_WEB_WIDTH;
    if x < edge {
        return None;
    }
    edge += AGE_WIDTH;
    (x < edge).then_some(if current == SortPreset::AgeNewest {
        SortPreset::AgeOldest
    } else {
        SortPreset::AgeNewest
    })
}

fn paint_header_cell(ui: &mut egui::Ui, rect: egui::Rect, text: &str, align: CellAlign) {
    paint_text(
        ui,
        rect,
        text,
        align,
        ui.visuals().strong_text_color(),
        egui::TextStyle::Button.resolve(ui.style()),
    );
}

fn draw_process_row(
    ui: &mut egui::Ui,
    process: &ProcessInfo,
    row: usize,
    selected: bool,
    advanced: bool,
    selected_pid: &mut Option<u32>,
    table_action: &mut Option<TableAction>,
) {
    let width = table_width(advanced).max(ui.available_width());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
    let row_fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else if row.is_multiple_of(2) {
        ui.visuals().faint_bg_color
    } else {
        ui.visuals().extreme_bg_color
    };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0.0, row_fill);

    let body_font = egui::TextStyle::Body.resolve(ui.style());
    let mono_font = egui::TextStyle::Monospace.resolve(ui.style());
    let text_color = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    let link_color = ui.visuals().hyperlink_color;
    let mut x = rect.left();

    paint_scope_badge(
        ui,
        next_rect(rect, &mut x, SCOPE_WIDTH),
        process.scope,
        selected,
    );
    paint_text(
        ui,
        next_rect(rect, &mut x, PID_WIDTH),
        &process.pid.to_string(),
        CellAlign::Right,
        text_color,
        body_font.clone(),
    );
    paint_text(
        ui,
        next_rect(rect, &mut x, NAME_WIDTH),
        &process.name,
        CellAlign::Left,
        text_color,
        body_font.clone(),
    );
    paint_text(
        ui,
        next_rect(rect, &mut x, RAM_WIDTH),
        &formatter::bytes_with_delta_mb_text(process.ram_bytes, process.ram_delta_bytes),
        CellAlign::Right,
        text_color,
        body_font.clone(),
    );
    paint_text(
        ui,
        next_rect(rect, &mut x, VRAM_WIDTH),
        &formatter::optional_bytes_with_delta_mb_text(
            process.vram_bytes(),
            process.vram_delta_bytes,
        ),
        CellAlign::Right,
        text_color,
        body_font.clone(),
    );

    let local_web_rect = next_rect(rect, &mut x, LOCAL_WEB_WIDTH);
    let local_web_text = process.local_web_table_text();
    let local_web_url = process.primary_local_web_url();
    paint_text(
        ui,
        local_web_rect,
        &local_web_text,
        CellAlign::Left,
        if local_web_url.is_some() {
            link_color
        } else {
            text_color
        },
        body_font.clone(),
    );

    paint_text(
        ui,
        next_rect(rect, &mut x, AGE_WIDTH),
        &formatter::age_text(process.start_time),
        CellAlign::Left,
        text_color,
        body_font.clone(),
    );
    if advanced {
        paint_text(
            ui,
            next_rect(rect, &mut x, PARENT_PID_WIDTH),
            &process
                .parent_pid
                .map(|pid| pid.to_string())
                .unwrap_or_default(),
            CellAlign::Right,
            text_color,
            body_font.clone(),
        );
        paint_text(
            ui,
            next_rect(rect, &mut x, PARENT_NAME_WIDTH),
            process.parent_name.as_deref().unwrap_or_default(),
            CellAlign::Left,
            text_color,
            body_font.clone(),
        );
        paint_text(
            ui,
            next_rect(rect, &mut x, PATH_WIDTH),
            process.exe_path.as_deref().unwrap_or_default(),
            CellAlign::Left,
            text_color,
            mono_font.clone(),
        );
        paint_text(
            ui,
            next_rect(rect, &mut x, COMMAND_WIDTH),
            process.command_line.as_deref().unwrap_or_default(),
            CellAlign::Left,
            text_color,
            mono_font,
        );
    }

    let hovered_local_web = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|pos| local_web_rect.contains(pos));
    let response = if hovered_local_web && local_web_url.is_some() {
        response.on_hover_text(process.local_web_summary())
    } else if process.ram_delta_bytes.is_some() || process.vram_delta_bytes.is_some() {
        response.on_hover_text(format!(
            "Snapshot state: {}\nRAM delta: {} MB\nVRAM delta: {} MB",
            process.snapshot_state,
            formatter::optional_delta_mb_text(process.ram_delta_bytes),
            formatter::optional_delta_mb_text(process.vram_delta_bytes)
        ))
    } else {
        response
    };

    if response.double_clicked() {
        *selected_pid = Some(process.pid);
        if let Some(url) = local_web_url {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    } else if response.clicked() {
        *selected_pid = Some(process.pid);
        if hovered_local_web {
            if let Some(url) = local_web_url {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
    }
    attach_context_menu(response, process, table_action);
}

fn table_width(advanced: bool) -> f32 {
    let compact =
        SCOPE_WIDTH + PID_WIDTH + NAME_WIDTH + RAM_WIDTH + VRAM_WIDTH + LOCAL_WEB_WIDTH + AGE_WIDTH;
    if advanced {
        compact + PARENT_PID_WIDTH + PARENT_NAME_WIDTH + PATH_WIDTH + COMMAND_WIDTH
    } else {
        compact
    }
}

fn next_rect(row: egui::Rect, x: &mut f32, width: f32) -> egui::Rect {
    let rect = egui::Rect::from_min_size(egui::pos2(*x, row.top()), egui::vec2(width, ROW_HEIGHT));
    *x += width;
    rect
}

fn paint_text(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &str,
    align: CellAlign,
    color: egui::Color32,
    font: egui::FontId,
) {
    let clip_rect = rect.shrink2(egui::vec2(2.0, 0.0));
    let painter = ui.painter().with_clip_rect(clip_rect);
    let (pos, anchor) = match align {
        CellAlign::Left => (
            egui::pos2(rect.left() + CELL_PAD_X, rect.center().y),
            egui::Align2::LEFT_CENTER,
        ),
        CellAlign::Right => (
            egui::pos2(rect.right() - CELL_PAD_X, rect.center().y),
            egui::Align2::RIGHT_CENTER,
        ),
    };
    painter.text(pos, anchor, text, font, color);
}

fn paint_scope_badge(ui: &mut egui::Ui, rect: egui::Rect, scope: ProcessScope, selected: bool) {
    let (fill, stroke, text_color) = scope_badge_style(scope, selected);
    let label = scope.as_str();
    let badge_width = ((label.chars().count() as f32 * 7.0) + 18.0).min(rect.width() - 12.0);
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + CELL_PAD_X, rect.top() + 5.0),
        egui::vec2(badge_width.max(44.0), ROW_HEIGHT - 10.0),
    );
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect(badge_rect, 4.0, fill, stroke);
    painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Small.resolve(ui.style()),
        text_color,
    );
}

fn scope_badge_style(
    scope: ProcessScope,
    selected: bool,
) -> (egui::Color32, egui::Stroke, egui::Color32) {
    let (fill, text) = match scope {
        ProcessScope::CodexGpu => (
            egui::Color32::from_rgb(211, 232, 245),
            egui::Color32::from_rgb(28, 73, 101),
        ),
        ProcessScope::CodexTerminal => (
            egui::Color32::from_rgb(229, 224, 246),
            egui::Color32::from_rgb(66, 55, 112),
        ),
        ProcessScope::Python => (
            egui::Color32::from_rgb(219, 239, 222),
            egui::Color32::from_rgb(48, 91, 55),
        ),
        ProcessScope::GpuActive => (
            egui::Color32::from_rgb(218, 238, 240),
            egui::Color32::from_rgb(42, 91, 94),
        ),
        ProcessScope::Protected => (
            egui::Color32::from_rgb(242, 224, 224),
            egui::Color32::from_rgb(111, 44, 44),
        ),
        ProcessScope::Normal => (
            egui::Color32::from_rgb(232, 234, 236),
            egui::Color32::from_rgb(72, 76, 80),
        ),
    };
    let stroke = if selected {
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 120, 150))
    } else {
        egui::Stroke::new(1.0_f32, fill)
    };
    (fill, stroke, text)
}

fn attach_context_menu(
    response: egui::Response,
    process: &ProcessInfo,
    table_action: &mut Option<TableAction>,
) {
    response.context_menu(|ui| {
        draw_process_context_menu(ui, process, table_action);
    });
}

fn draw_process_context_menu(
    ui: &mut egui::Ui,
    process: &ProcessInfo,
    table_action: &mut Option<TableAction>,
) {
    if let Some(url) = process.primary_local_web_url() {
        if ui.button("Open Local Web").clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            ui.close_menu();
        }
        if ui.button("Copy URL").clicked() {
            ui.ctx().copy_text(url.to_string());
            ui.close_menu();
        }
        if process.local_web_port_count() > 1 && ui.button("Copy All URLs").clicked() {
            ui.ctx().copy_text(process.local_web_summary());
            ui.close_menu();
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Open Local Web"));
        ui.add_enabled(false, egui::Button::new("Copy URL"));
    }

    ui.separator();
    if ui.button("Copy PID").clicked() {
        ui.ctx().copy_text(process.pid.to_string());
        ui.close_menu();
    }
    if ui.button("Copy Process Name").clicked() {
        ui.ctx().copy_text(process.name.clone());
        ui.close_menu();
    }
    if let Some(path) = &process.exe_path {
        if ui.button("Copy Path").clicked() {
            ui.ctx().copy_text(path.clone());
            ui.close_menu();
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Copy Path"));
    }
    if let Some(command_line) = &process.command_line {
        if ui.button("Copy Command Line").clicked() {
            ui.ctx().copy_text(command_line.clone());
            ui.close_menu();
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Copy Command Line"));
    }
    if let Some(cwd) = &process.cwd {
        if ui.button("Copy CWD").clicked() {
            ui.ctx().copy_text(cwd.clone());
            ui.close_menu();
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Copy CWD"));
    }

    ui.separator();
    if process.protected {
        ui.add_enabled(false, egui::Button::new("Close"));
        ui.add_enabled(false, egui::Button::new("Kill"));
        ui.add_enabled(false, egui::Button::new("Kill Tree"));
    } else {
        if ui.button("Close").clicked() {
            *table_action = Some(TableAction::Close(process.pid));
            ui.close_menu();
        }
        if ui.button("Kill").clicked() {
            *table_action = Some(TableAction::Kill(process.pid));
            ui.close_menu();
        }
        if ui.button("Kill Tree").clicked() {
            *table_action = Some(TableAction::KillTree(process.pid));
            ui.close_menu();
        }
    }
}
