mod app;
mod collectors;
mod fonts;
mod model;
mod services;
mod settings;
mod ui;

fn main() -> eframe::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["--version"] {
        println!("RunScope {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.as_slice() == ["--self-check"] {
        match services::self_check::run() {
            Ok(report) => {
                println!("{report}");
                return Ok(());
            }
            Err(error) => {
                eprintln!("RunScope self-check failed: {error:#}");
                std::process::exit(1);
            }
        }
    }
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        print_usage();
        return Ok(());
    }

    let mut load_on_start = false;
    let mut screenshot_path = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--load" => {
                load_on_start = true;
            }
            "--screenshot" => {
                index += 1;
                let Some(path) = args.get(index) else {
                    eprintln!("--screenshot requires a .bmp output path");
                    print_usage();
                    std::process::exit(2);
                };
                let path = std::path::PathBuf::from(path);
                if !path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("bmp"))
                {
                    eprintln!("--screenshot output path must use the .bmp extension");
                    std::process::exit(2);
                }
                screenshot_path = Some(path);
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                print_usage();
                std::process::exit(2);
            }
        }
        index += 1;
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
        Box::new(move |cc| {
            fonts::install_japanese_fonts(&cc.egui_ctx);
            let mut app = app::WatcherApp::default();
            if let Some(path) = screenshot_path {
                app.set_screenshot_path(path);
            }
            if load_on_start {
                app.start_load(&cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    )
}

fn print_usage() {
    println!("RunScope {}", env!("CARGO_PKG_VERSION"));
    println!("Usage: RunScope.exe [--load] [--screenshot <output.bmp>]");
    println!("       RunScope.exe --version");
    println!("       RunScope.exe --self-check");
}
