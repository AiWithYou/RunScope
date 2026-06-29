use std::path::Path;

pub fn install_japanese_fonts(ctx: &egui::Context) {
    let Some(bytes) = read_japanese_font() else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "runscope_japanese".to_string(),
        egui::FontData::from_owned(bytes),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "runscope_japanese".to_string());
    }

    ctx.set_fonts(fonts);
}

fn read_japanese_font() -> Option<Vec<u8>> {
    [
        r"C:\Windows\Fonts\NotoSansJP-VF.ttf",
        r"C:\Windows\Fonts\YuGothR.ttc",
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
    ]
    .iter()
    .find_map(|path| std::fs::read(Path::new(path)).ok())
}
