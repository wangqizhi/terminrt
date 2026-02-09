use egui::{Align, Color32, Layout, RichText};

const LEFT_PANEL_WIDTH: f32 = 260.0;

pub fn render(ctx: &egui::Context, devtools_open: &mut bool) {
    let panel_stroke = egui::Stroke::new(1.0, Color32::from_gray(70));
    let side_fill = Color32::from_gray(18);

    egui::SidePanel::left("left_panel")
        .resizable(false)
        .exact_width(LEFT_PANEL_WIDTH)
        .frame(egui::Frame::none().fill(side_fill).stroke(panel_stroke))
        .show(ctx, |ui| {
            let panel_rect = ui.max_rect();
            let header_h = 56.0;
            let footer_h = 40.0;

            let header_rect = egui::Rect::from_min_size(
                panel_rect.min,
                egui::vec2(panel_rect.width(), header_h),
            );
            let footer_rect = egui::Rect::from_min_size(
                egui::pos2(panel_rect.left(), panel_rect.bottom() - footer_h),
                egui::vec2(panel_rect.width(), footer_h),
            );

            ui.allocate_ui_at_rect(header_rect, |ui| {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("TERMINRT")
                            .monospace()
                            .size(18.0)
                            .color(Color32::from_gray(220)),
                    );
                });
            });

            ui.allocate_ui_at_rect(footer_rect, |ui| {
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.add_space(6.0);
                    let label = if *devtools_open { "DevTools ▶" } else { "DevTools ◀" };
                    let btn = ui.add(
                        egui::Button::new(
                            RichText::new(label)
                                .monospace()
                                .size(11.0)
                                .color(Color32::from_gray(160)),
                        )
                        .frame(false),
                    );
                    if btn.clicked() {
                        *devtools_open = !*devtools_open;
                    }
                });
            });
        });
}
