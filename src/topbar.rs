use egui::{Align, Color32, FontId, Layout, RichText, Sense, Stroke};

pub struct TopBarInput<'a> {
    pub terminal_exited: bool,
    pub terminal_connecting: bool,
    pub reconnect_requested: &'a mut bool,
}

#[derive(Default, Clone, Copy)]
pub struct TopBarAction {
    pub request_minimize: bool,
    pub request_toggle_maximize: bool,
    pub request_close: bool,
    pub request_drag_window: bool,
}

pub fn render(ui: &mut egui::Ui, input: TopBarInput<'_>, bar_color: Color32) -> TopBarAction {
    let mut action = TopBarAction::default();
    let bar_rect = ui.max_rect();

    // Background fill for the bar itself.
    ui.painter().rect_filled(bar_rect, 0.0, bar_color);

    let buttons_w = 3.0 * 18.0 + 2.0 * 6.0 + 8.0;
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.right() - buttons_w, bar_rect.top()),
        egui::vec2(buttons_w, bar_rect.height()),
    );
    let left_rect = egui::Rect::from_min_size(
        bar_rect.min,
        egui::vec2((bar_rect.width() - buttons_w).max(0.0), bar_rect.height()),
    );

    let drag_response = ui.interact(
        left_rect,
        egui::Id::new("topbar_drag_area"),
        Sense::click_and_drag(),
    );
    if drag_response.drag_started() {
        action.request_drag_window = true;
    }
    if drag_response.double_clicked() {
        action.request_toggle_maximize = true;
    }

    ui.allocate_ui_at_rect(left_rect, |ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if input.terminal_exited {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("PowerShell exited")
                        .monospace()
                        .color(Color32::from_gray(190))
                        .size(12.0),
                );
                ui.add_space(8.0);
                let reconnect = ui.add_enabled(
                    !input.terminal_connecting,
                    egui::Button::new(RichText::new("Reconnect").monospace().size(12.0))
                        .min_size(egui::vec2(92.0, 18.0)),
                );
                if reconnect.clicked() {
                    *input.reconnect_requested = true;
                }
                if input.terminal_connecting {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Reconnecting...")
                            .monospace()
                            .color(Color32::from_gray(150))
                            .size(12.0),
                    );
                }
            }
        });
    });

    ui.allocate_ui_at_rect(right_rect, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let close_button = egui::Button::new(
                RichText::new("X")
                    .font(FontId::monospace(11.0))
                    .color(Color32::from_gray(230)),
            )
            .fill(Color32::from_rgb(150, 50, 50))
            .stroke(Stroke::new(1.0, Color32::from_gray(70)));
            if ui.add_sized(egui::vec2(18.0, 18.0), close_button).clicked() {
                action.request_close = true;
            }

            let max_button = egui::Button::new(
                RichText::new("[]")
                    .font(FontId::monospace(10.0))
                    .color(Color32::from_gray(210)),
            )
            .fill(Color32::from_gray(35))
            .stroke(Stroke::new(1.0, Color32::from_gray(70)));
            if ui.add_sized(egui::vec2(18.0, 18.0), max_button).clicked() {
                action.request_toggle_maximize = true;
            }

            let min_button = egui::Button::new(
                RichText::new("-")
                    .font(FontId::monospace(12.0))
                    .color(Color32::from_gray(210)),
            )
            .fill(Color32::from_gray(35))
            .stroke(Stroke::new(1.0, Color32::from_gray(70)));
            if ui.add_sized(egui::vec2(18.0, 18.0), min_button).clicked() {
                action.request_minimize = true;
            }
        });
    });

    action
}
