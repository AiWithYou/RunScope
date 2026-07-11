use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context};

pub fn save_bmp(image: &egui::ColorImage, path: &Path) -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    write_bmp(image, &mut bytes)?;
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.to_string_lossy()))?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

fn write_bmp(image: &egui::ColorImage, writer: &mut impl Write) -> anyhow::Result<()> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || image.pixels.len() != width.saturating_mul(height) {
        bail!("invalid screenshot dimensions");
    }
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("screenshot is too large")?;
    let file_size = 54_usize
        .checked_add(pixel_bytes)
        .context("screenshot file size overflowed")?;
    let file_size = u32::try_from(file_size).context("screenshot exceeds BMP size limit")?;
    let width = i32::try_from(width).context("screenshot width exceeds BMP limit")?;
    let height = i32::try_from(height).context("screenshot height exceeds BMP limit")?;

    writer.write_all(b"BM")?;
    writer.write_all(&file_size.to_le_bytes())?;
    writer.write_all(&[0; 4])?;
    writer.write_all(&54_u32.to_le_bytes())?;
    writer.write_all(&40_u32.to_le_bytes())?;
    writer.write_all(&width.to_le_bytes())?;
    writer.write_all(&(-height).to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&32_u16.to_le_bytes())?;
    writer.write_all(&0_u32.to_le_bytes())?;
    writer.write_all(&(file_size - 54).to_le_bytes())?;
    writer.write_all(&[0; 16])?;
    for pixel in &image.pixels {
        writer.write_all(&[pixel.b(), pixel.g(), pixel.r(), 255])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_top_down_bgra_bmp() {
        let image = egui::ColorImage {
            size: [1, 1],
            pixels: vec![egui::Color32::from_rgb(1, 2, 3)],
        };
        let mut bytes = Vec::new();
        write_bmp(&image, &mut bytes).unwrap();

        assert_eq!(&bytes[..2], b"BM");
        assert_eq!(&bytes[54..58], &[3, 2, 1, 255]);
    }
}
