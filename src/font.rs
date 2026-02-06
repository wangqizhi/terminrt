use std::fs;

pub struct FontRasterizer {
    font: fontdue::Font,
}

impl FontRasterizer {
    pub fn load_system() -> Self {
        // Try a small set of common system font locations for portability.
        let candidates = system_font_candidates();
        let mut last_err = None;
        for path in candidates {
            match fs::read(&path) {
                Ok(bytes) => {
                    match fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()) {
                        Ok(font) => return Self { font },
                        Err(err) => {
                            last_err = Some(format!("Font parse failed for {}: {}", path, err));
                        }
                    }
                }
                Err(err) => {
                    last_err = Some(format!("Font read failed for {}: {}", path, err));
                }
            }
        }

        panic!(
            "Failed to load any system font. Last error: {}",
            last_err.unwrap_or_else(|| "no candidates tried".to_string())
        );
    }

    pub fn rasterize(&self, ch: char, size_px: f32) -> (fontdue::Metrics, Vec<u8>) {
        self.font.rasterize(ch, size_px)
    }
}

fn system_font_candidates() -> Vec<String> {
    let mut paths = Vec::new();

    // Windows common fonts
    paths.push("C:\\Windows\\Fonts\\arial.ttf".to_string());
    paths.push("C:\\Windows\\Fonts\\arialbd.ttf".to_string());
    paths.push("C:\\Windows\\Fonts\\consola.ttf".to_string());
    paths.push("C:\\Windows\\Fonts\\segoeui.ttf".to_string());

    // macOS common fonts
    paths.push("/System/Library/Fonts/SFNS.ttf".to_string());
    paths.push("/System/Library/Fonts/Supplemental/Arial.ttf".to_string());
    paths.push("/System/Library/Fonts/Supplemental/Courier New.ttf".to_string());

    // Linux common fonts
    paths.push("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string());
    paths.push("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf".to_string());
    paths.push("/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf".to_string());

    paths
}
