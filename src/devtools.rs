use egui;
use crate::terminal;
use crate::quickcmd::{self, QuickCommandConfig};
use crate::settings::SettingsState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DevToolsTab {
    QuickCommands,
    VtStream,
    Network,
}

/// Describes a quick command the user clicked in the panel.
pub struct QuickCmdAction {
    pub command: String,
    pub auto_execute: bool,
}

pub struct DevToolsState {
    pub active_tab: DevToolsTab,
    /// Tag currently selected for filtering quick commands in the panel.
    pub qcmd_filter_tag: String,
}

impl Default for DevToolsState {
    fn default() -> Self {
        Self {
            active_tab: DevToolsTab::QuickCommands,
            qcmd_filter_tag: String::new(),
        }
    }
}

pub fn render_devtools(
    ctx: &egui::Context,
    state: &mut DevToolsState,
    terminal: Option<&terminal::TerminalInstance>,
    qcmd_config: &QuickCommandConfig,
    settings_state: &mut SettingsState,
    width: f32,
) -> Option<QuickCmdAction> {
    let side_fill = egui::Color32::from_rgb(30, 30, 30);
    let panel_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(60));
    let mut action: Option<QuickCmdAction> = None;

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
                ui.selectable_value(&mut state.active_tab, DevToolsTab::QuickCommands, "⚡ Cmds");
                ui.selectable_value(&mut state.active_tab, DevToolsTab::VtStream, "VT Stream");
                ui.selectable_value(&mut state.active_tab, DevToolsTab::Network, "Network");
            });
            ui.separator();

            match state.active_tab {
                DevToolsTab::QuickCommands => {
                    action = render_quick_commands_panel(ui, state, qcmd_config, settings_state);
                }
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

    action
}

// ---------------------------------------------------------------------------
// Quick commands panel in the right sidebar
// ---------------------------------------------------------------------------

fn render_quick_commands_panel(
    ui: &mut egui::Ui,
    state: &mut DevToolsState,
    config: &QuickCommandConfig,
    settings_state: &mut SettingsState,
) -> Option<QuickCmdAction> {
    let mut action: Option<QuickCmdAction> = None;
    let tags = config.tags();

    // Header: tag filter buttons + settings "+" button
    ui.horizontal_wrapped(|ui| {
        ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 3.0);
        // "All" tag
        let all_sel = state.qcmd_filter_tag.is_empty();
        if ui
            .selectable_label(
                all_sel,
                egui::RichText::new("All").monospace().size(11.0),
            )
            .clicked()
        {
            state.qcmd_filter_tag.clear();
        }
        for tag in &tags {
            let sel = state.qcmd_filter_tag == *tag;
            if ui
                .selectable_label(
                    sel,
                    egui::RichText::new(tag).monospace().size(11.0),
                )
                .clicked()
            {
                if sel {
                    state.qcmd_filter_tag.clear();
                } else {
                    state.qcmd_filter_tag = tag.clone();
                }
            }
        }

        // "+" button → open settings
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("+")
                        .monospace()
                        .size(14.0)
                        .color(egui::Color32::WHITE)
                        .strong(),
                )
                .fill(egui::Color32::from_rgb(45, 125, 235))
                .min_size(egui::vec2(22.0, 18.0))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 160, 255))),
            )
            .on_hover_text("Configure quick commands")
            .clicked()
        {
            settings_state.open = true;
        }
    });

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);

    // Filter commands
    let commands: Vec<&quickcmd::QuickCommand> = if state.qcmd_filter_tag.is_empty() {
        config.commands.iter().collect()
    } else {
        config
            .commands
            .iter()
            .filter(|c| c.tag == state.qcmd_filter_tag)
            .collect()
    };

    if commands.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No quick commands")
                    .color(egui::Color32::from_gray(110))
                    .italics()
                    .size(12.0),
            );
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("＋ Add")
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::WHITE),
                    )
                    .fill(egui::Color32::from_rgb(45, 125, 235)),
                )
                .clicked()
            {
                settings_state.open = true;
            }
        });
    } else {
        // Group by tag
        let display_tags: Vec<String> = if state.qcmd_filter_tag.is_empty() {
            config.tags()
        } else {
            vec![state.qcmd_filter_tag.clone()]
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for tag in &display_tags {
                    let tag_cmds: Vec<&&quickcmd::QuickCommand> =
                        commands.iter().filter(|c| c.tag == *tag).collect();
                    if tag_cmds.is_empty() {
                        continue;
                    }

                    // Tag header
                    ui.horizontal(|ui| {
                        let badge = egui::Frame::none()
                            .fill(egui::Color32::from_rgb(50, 60, 80))
                            .rounding(egui::Rounding::same(3.0))
                            .inner_margin(egui::Margin::symmetric(5.0, 1.0));
                        badge.show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(tag)
                                    .monospace()
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(140, 180, 255)),
                            );
                        });
                    });
                    ui.add_space(2.0);

                    // Command buttons in a flow layout
                    ui.horizontal_wrapped(|ui| {
                        ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 4.0);
                        for cmd in &tag_cmds {
                            let btn_text = if cmd.keybinding.is_empty() {
                                cmd.name.clone()
                            } else {
                                format!("{} [{}]", cmd.name, cmd.keybinding.display())
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(&btn_text)
                                    .monospace()
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(220)),
                            )
                            .fill(egui::Color32::from_gray(40))
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(65)))
                            .rounding(egui::Rounding::same(4.0));

                            let resp = ui.add(btn).on_hover_text(&cmd.command);
                            if resp.clicked() {
                                action = Some(QuickCmdAction {
                                    command: cmd.command.clone(),
                                    auto_execute: cmd.auto_execute,
                                });
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
            });
    }

    action
}
