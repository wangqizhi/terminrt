use egui;
use crate::terminal;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DevToolsTab {
    VtStream,
    Network,
}

pub struct DevToolsState {
    pub active_tab: DevToolsTab,
}

impl Default for DevToolsState {
    fn default() -> Self {
        Self {
            active_tab: DevToolsTab::VtStream,
        }
    }
}

pub fn render_devtools(
    ctx: &egui::Context,
    state: &mut DevToolsState,
    terminal: Option<&terminal::TerminalInstance>,
    width: f32,
) {
    let side_fill = egui::Color32::from_rgb(30, 30, 30);
    let panel_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(60));

    egui::SidePanel::right("right_panel")
        .resizable(false)
        .exact_width(width)
        .frame(egui::Frame::none().fill(side_fill).stroke(panel_stroke))
        .show(ctx, |ui| {
            ui.add_space(6.0);
            
            // Tabs
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 15.0;
                ui.add_space(6.0);
                ui.selectable_value(&mut state.active_tab, DevToolsTab::VtStream, "VT Stream");
                ui.selectable_value(&mut state.active_tab, DevToolsTab::Network, "Network");
            });
            ui.separator();

            match state.active_tab {
                DevToolsTab::VtStream => {
                    terminal::render_vt_log(ui, terminal);
                }
                DevToolsTab::Network => {
                     ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Under Development")
                                .color(egui::Color32::from_gray(120))
                                .italics()
                        );
                    });
                }
            }
        });
}
