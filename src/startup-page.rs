use std::time::Instant;

const TEXT: &str = "HELLO TERMINRT!";
const CHAR_STEP_SECS: f32 = 0.12;
const CHAR_FADE_SECS: f32 = 0.26;
const END_HOLD_SECS: f32 = 0.16;

fn animation_total_secs() -> f32 {
    let char_count = TEXT.chars().count();
    if char_count == 0 {
        return 0.0;
    }
    (char_count.saturating_sub(1) as f32 * CHAR_STEP_SECS) + CHAR_FADE_SECS + END_HOLD_SECS
}

pub fn is_animation_done(elapsed_secs: f32) -> bool {
    elapsed_secs >= animation_total_secs()
}

pub fn render(ui: &mut egui::Ui, started_at: Instant, error: Option<&str>) {
    let elapsed = started_at.elapsed().as_secs_f32();
    if !is_animation_done(elapsed) {
        ui.ctx().request_repaint();
    }

    let rect = ui.max_rect();
    let center = rect.center();
    let bar_width = (rect.width() * 0.62).clamp(360.0, 980.0);
    let bar_height = 92.0;
    let bar_rect = egui::Rect::from_center_size(center, egui::vec2(bar_width, bar_height));

    ui.painter()
        .rect_filled(bar_rect, 10.0, egui::Color32::from_rgb(0, 0, 0));
    ui.painter().rect_stroke(
        bar_rect,
        10.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(42)),
    );

    let chars: Vec<char> = TEXT.chars().collect();
    let char_count = chars.len();
    let char_advance = (bar_width / 14.0).clamp(22.0, 42.0);
    let text_left = center.x - (char_count.saturating_sub(1) as f32 * char_advance) * 0.5;
    let text_y = center.y;

    for (idx, ch) in chars.iter().enumerate() {
        let t = elapsed - idx as f32 * CHAR_STEP_SECS;
        let alpha = (t / CHAR_FADE_SECS).clamp(0.0, 1.0);
        let color = egui::Color32::from_rgba_unmultiplied(236, 241, 248, (alpha * 255.0) as u8);
        let x = text_left + idx as f32 * char_advance;
        ui.painter().text(
            egui::pos2(x, text_y),
            egui::Align2::CENTER_CENTER,
            ch,
            egui::FontId::monospace(42.0),
            color,
        );
    }

    let status = if let Some(err) = error {
        format!("PTY start failed: {}", err)
    } else {
        "Initializing terminal...".to_string()
    };
    let status_color = if error.is_some() {
        egui::Color32::from_rgb(220, 90, 90)
    } else {
        egui::Color32::from_gray(145)
    };
    ui.painter().text(
        egui::pos2(center.x, bar_rect.bottom() + 22.0),
        egui::Align2::CENTER_CENTER,
        status,
        egui::FontId::monospace(13.0),
        status_color,
    );
}
