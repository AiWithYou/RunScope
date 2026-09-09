use super::{worker, *};
use crate::app::WatcherApp;
use std::path::PathBuf;
use std::time::Duration;

/// Composition keeps recording state independent of inspector filters and termination actions.
pub struct DiagnosticsApp {
    inspector: WatcherApp,
    show_recorder: bool,
    config: Config,
    mode_index: u8,
    output_root: String,
    run: Option<worker::Run>,
    error: Option<String>,
    exit_requested: bool,
}

impl DiagnosticsApp {
    pub fn new(inspector: WatcherApp) -> Self {
        Self {
            inspector,
            show_recorder: false,
            config: Config::default(),
            mode_index: 0,
            output_root: worker::default_root().to_string_lossy().into_owned(),
            run: None,
            error: None,
            exit_requested: false,
        }
    }

    fn active(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.active())
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.error = None;
        let mut config = self.config.clone();
        config.mode = match self.mode_index {
            1 => match self.inspector.selected_process().and_then(ProcessKey::of) {
                Some(root) => CaptureMode::Tree { root },
                None => {
                    self.error = Some(
                        "Load and select a process with a verified creation time first.".into(),
                    );
                    return;
                }
            },
            2 => CaptureMode::All,
            _ => CaptureMode::AiWorkloads,
        };
        if self.output_root.trim().is_empty() {
            self.error = Some("Choose a recording directory.".into());
            return;
        }
        match worker::Run::start(
            &PathBuf::from(&self.output_root),
            config,
            self.inspector.settings.clone(),
            Some(ctx.clone()),
        ) {
            Ok(run) => self.run = Some(run),
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }

    fn window(&mut self, ctx: &egui::Context) {
        let active = self.active();
        let mut open = self.show_recorder;
        egui::Window::new("Flight recorder · RunScope 2").open(&mut open)
            .default_size([790.0, 580.0]).resizable(true).show(ctx, |ui| {
                ui.label("Record an AI workload without changing the process inspector. No automatic termination.");
                ui.add_enabled_ui(!active, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.mode_index, 0, "AI workloads + children");
                        ui.selectable_value(&mut self.mode_index, 1, "Selected process tree");
                        ui.selectable_value(&mut self.mode_index, 2, "All processes");
                    });
                    if self.mode_index == 1 {
                        let selected = self.inspector.selected_process().map(|p| format!("{} · {}", p.pid, p.name)).unwrap_or_else(|| "No process selected".into());
                        ui.label(format!("Root: {selected}. Existing descendants keep being followed after root exit."));
                    }
                    ui.horizontal(|ui| {
                        ui.label("Interval (seconds)");
                        ui.add(egui::DragValue::new(&mut self.config.interval_seconds).range(1..=60));
                        ui.label("Log limit (MiB)");
                        let mut mib = self.config.max_log_bytes / 1048576;
                        if ui.add(egui::DragValue::new(&mut mib).range(1..=4096)).changed() {
                            self.config.max_log_bytes = mib * 1048576;
                        }
                    });
                    ui.horizontal(|ui| { ui.label("Save under"); ui.text_edit_singleline(&mut self.output_root); });
                    ui.checkbox(&mut self.config.include_sensitive, "Include executable paths, command lines and working directories (may contain secrets)");
                    ui.small("Default exports still include process names, PIDs, memory values and port numbers. Review before sharing.");
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(!active, egui::Button::new("Start recording")).clicked() { self.start(ctx); }
                    if ui.add_enabled(active, egui::Button::new("Stop and write reports")).clicked() {
                        if let Some(run) = &mut self.run { run.stop(); }
                    }
                    if let Some(run) = &self.run {
                        if ui.button("Open folder").clicked() {
                            if let Err(e) = open_path(&run.directory) { self.error = Some(e); }
                        }
                        if !active && run.directory.join("report.html").is_file() && ui.button("Open HTML report").clicked() {
                            if let Err(e) = open_path(&run.directory.join("report.html")) { self.error = Some(e); }
                        }
                    }
                });
                if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                if let Some(run) = &self.run {
                    if let Some(outcome) = &run.outcome {
                        match outcome {
                            Ok(reason) => { ui.label(format!("Finished: {reason}")); }
                            Err(error) => { ui.colored_label(ui.visuals().error_fg_color, format!("{error}\nRecover samples.jsonl with --report.")); }
                        }
                    } else if run.stopping { ui.label("Finishing the current sample and writing reports..."); }
                    let p = &run.progress;
                    ui.separator();
                    ui.label(format!("{} samples · {:.1}s · {} processes · last collection {}ms · log {:.1} MiB",
                        p.samples, p.elapsed_ms as f64 / 1000.0, p.totals.process_count, p.collection_ms, p.log_bytes as f64 / 1048576.0));
                    let vram = p.totals.vram_bytes_known.map(|v| format!("{:.1} MiB", v as f64 / 1048576.0)).unwrap_or_else(|| "N/A".into());
                    let cpu = p.totals.cpu_percent_known.map(|v| format!("{v:.1}%")).unwrap_or_else(|| "N/A".into());
                    ui.label(format!("RAM sum {:.1} MiB · known VRAM {vram} ({}/{}) · known CPU {cpu} ({}/{})",
                        p.totals.ram_bytes as f64 / 1048576.0, p.totals.vram_known_count, p.totals.process_count,
                        p.totals.cpu_known_count, p.totals.process_count));
                    ui.small("RAM is a sum of working sets; shared pages can be counted twice. CPU 100% = one logical CPU.");
                    draw_chart(ui, &p.points);
                    ui.small(&p.vram_status);
                    ui.label("Recent observations (not crash verdicts)");
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for event in &p.last_events {
                            let process = event.process.as_ref().map(|p| format!("PID {} {}", p.pid, p.name)).unwrap_or_default();
                            ui.label(format!("{:.1}s · {} · {process}", event.elapsed_ms as f64 / 1000.0, event.kind));
                            ui.small(&event.message);
                        }
                    });
                    ui.separator();
                    ui.label(run.directory.to_string_lossy());
                } else {
                    ui.separator();
                    ui.label("Start immediately before reproduction; stop after the slowdown or process disappearance.");
                    ui.label("Outputs: samples.jsonl (all samples), report.json, report.html (bounded summary).");
                    ui.small("The recorder is independent of inspector filters. Closing this window does not stop recording. Closing RunScope stops and finalizes it. A forced termination can be recovered from complete JSONL records.");
                }
            });
        self.show_recorder = open;
    }
}

impl eframe::App for DiagnosticsApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(run) = &mut self.run {
            run.poll();
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.active() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.exit_requested = true;
            if let Some(run) = &mut self.run {
                run.stop();
            }
        }
        if self.exit_requested && !self.active() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::TopBottomPanel::top("flight_recorder_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("RunScope 2");
                if ui.button(if self.active() { "Recording · open recorder" } else { "Flight recorder" }).clicked() { self.show_recorder = true; }
                if self.active() {
                    if ui.button("Stop recording").clicked() { if let Some(run) = &mut self.run { run.stop(); } }
                    ui.small("Independent sampling; opening/closing the recorder window does not affect recording.");
                } else { ui.small("Opt-in RAM / VRAM / CPU / I/O history and recoverable reports"); }
            });
        });
        self.inspector.update(ctx, frame);
        if self.show_recorder {
            self.window(ctx);
        }
        // Repaint only while recording; no extra idle timer. Collection runs on its own thread.
        if self.active() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

fn open_path(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Cannot open {}: {e}", path.display()))
}

fn draw_chart(ui: &mut egui::Ui, points: &[Point]) {
    ui.small("Recent working-set sum · MiB (up to 120 samples)");
    let (response, painter) =
        ui.allocate_painter(egui::vec2(ui.available_width(), 95.0), egui::Sense::hover());
    let rect = response.rect.shrink(5.0);
    painter.rect_filled(response.rect, 4.0, ui.visuals().extreme_bg_color);
    let max = points
        .iter()
        .map(|p| p.totals.ram_bytes)
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    let first = points.first().map_or(0, |p| p.elapsed_ms);
    let span = points
        .last()
        .map_or(0, |p| p.elapsed_ms)
        .saturating_sub(first)
        .max(1) as f32;
    let positions: Vec<_> = points
        .iter()
        .map(|p| {
            egui::pos2(
                rect.left() + p.elapsed_ms.saturating_sub(first) as f32 / span * rect.width(),
                rect.bottom() - p.totals.ram_bytes as f32 / max * rect.height(),
            )
        })
        .collect();
    let stroke = egui::Stroke::new(1.5, ui.visuals().selection.bg_fill);
    for pair in positions.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
    for position in positions {
        painter.circle_filled(position, 1.8, stroke.color);
    }
}
