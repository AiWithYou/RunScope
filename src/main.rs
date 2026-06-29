mod app;
mod collectors;
mod fonts;
mod model;
mod services;
mod settings;
mod ui;

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" => {
                println!("RunScope {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--self-check" => match services::self_check::run() {
                Ok(report) => {
                    println!("{report}");
                    return Ok(());
                }
                Err(error) => {
                    eprintln!("RunScope self-check failed: {error:#}");
                    std::process::exit(1);
                }
            },
            _ => {
                eprintln!("Unknown argument: {arg}");
                eprintln!("Usage: RunScope.exe [--version|--self-check]");
                std::process::exit(2);
            }
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1220.0, 720.0])
            .with_min_inner_size([900.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RunScope",
        options,
        Box::new(|cc| {
            fonts::install_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(app::WatcherApp::default()))
        }),
    )
}
