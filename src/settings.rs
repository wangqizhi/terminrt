use egui::{self, Color32, RichText, Stroke};
use crate::quickcmd::{KeyBinding, QuickCommand, QuickCommandConfig};

// ---------------------------------------------------------------------------
// Settings state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingsTab {
    QuickCommands,
}

pub struct SettingsState {
    pub open: bool,
    pub active_tab: SettingsTab,
    /// Tag currently selected for filtering in the settings list.
    pub filter_tag: String,
    /// Editing state: the command currently being edited (clone for form).
    pub editing: Option<QuickCommand>,
    /// True when we are creating a new command (vs editing existing).
    pub creating_new: bool,
    /// True when we are recording a keybinding.
    pub recording_keybinding: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            active_tab: SettingsTab::QuickCommands,
            filter_tag: String::new(),
            editing: None,
            creating_new: false,
            recording_keybinding: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Public render entry
// ---------------------------------------------------------------------------

/// Render the settings modal window. Returns true if the config was modified
/// (caller should persist).
pub fn render_settings(
    ctx: &egui::Context,
    settings: &mut SettingsState,
    config: &mut QuickCommandConfig,
) -> bool {
    if !settings.open {
        return false;
    }

    let mut dirty = false;

    // Dim background
    let screen_rect = ctx.screen_rect();
    let blocker_layer = egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new("settings_modal_blocker"),
    );
    ctx.layer_painter(blocker_layer).rect_filled(
        screen_rect,
        0.0,
        Color32::from_rgba_unmultiplied(0, 0, 0, 120),
    );

    let win_w = (screen_rect.width() * 0.72).min(820.0).max(480.0);
    let win_h = (screen_rect.height() * 0.78).min(640.0).max(360.0);
    let center = screen_rect.center();

    egui::Window::new("Settings")
        .id(egui::Id::new("settings_window"))
        .collapsible(false)
        .resizable(false)
        .fixed_size(egui::vec2(win_w, win_h))
        .default_pos(egui::pos2(center.x - win_w * 0.5, center.y - win_h * 0.5))
        .movable(true)
        .show(ctx, |ui| {
            // Tab row
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 12.0;
                ui.selectable_value(
                    &mut settings.active_tab,
                    SettingsTab::QuickCommands,
                    RichText::new("⚡ Quick Commands").monospace().size(13.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Close")
                                    .monospace()
                                    .size(12.0)
                                    .color(Color32::from_gray(180)),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        settings.open = false;
                        settings.editing = None;
                        settings.creating_new = false;
                    }
                });
            });
            ui.separator();

            match settings.active_tab {
                SettingsTab::QuickCommands => {
                    dirty = render_quick_commands_tab(ui, settings, config);
                }
            }
        });

    dirty
}

// ---------------------------------------------------------------------------
// Quick commands tab
// ---------------------------------------------------------------------------

fn render_quick_commands_tab(
    ui: &mut egui::Ui,
    settings: &mut SettingsState,
    config: &mut QuickCommandConfig,
) -> bool {
    // If we are editing a command, show the edit form; otherwise the list.
    if settings.editing.is_some() {
        render_edit_form(ui, settings, config)
    } else {
        render_command_list(ui, settings, config)
    }
}

// ---------------------------------------------------------------------------
// Command list with tag filter
// ---------------------------------------------------------------------------

fn render_command_list(
    ui: &mut egui::Ui,
    settings: &mut SettingsState,
    config: &mut QuickCommandConfig,
) -> bool {
    let mut dirty = false;
    let tags = config.tags();

    // Top toolbar: tag filter + add button
    ui.horizontal(|ui| {
        ui.label(RichText::new("Tag:").monospace().size(12.0).color(Color32::from_gray(160)));
        // "All" option
        let all_selected = settings.filter_tag.is_empty();
        if ui
            .selectable_label(all_selected, RichText::new("All").monospace().size(12.0))
            .clicked()
        {
            settings.filter_tag.clear();
        }
        for tag in &tags {
            let selected = settings.filter_tag == *tag;
            if ui
                .selectable_label(selected, RichText::new(tag).monospace().size(12.0))
                .clicked()
            {
                if selected {
                    settings.filter_tag.clear();
                } else {
                    settings.filter_tag = tag.clone();
                }
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("＋ Add Command")
                            .monospace()
                            .size(12.0)
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(45, 125, 235))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(90, 160, 255))),
                )
                .clicked()
            {
                settings.editing = Some(QuickCommand::new_empty());
                settings.creating_new = true;
            }
        });
    });

    ui.add_space(6.0);
    ui.separator();

    // Command list
    let commands: Vec<QuickCommand> = if settings.filter_tag.is_empty() {
        config.commands.clone()
    } else {
        config
            .commands
            .iter()
            .filter(|c| c.tag == settings.filter_tag)
            .cloned()
            .collect()
    };

    if commands.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No quick commands configured yet.")
                    .color(Color32::from_gray(120))
                    .italics()
                    .size(13.0),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("Click \"＋ Add Command\" to create one.")
                    .color(Color32::from_gray(100))
                    .size(12.0),
            );
        });
    } else {
        let mut remove_id: Option<String> = None;
        let mut edit_cmd: Option<QuickCommand> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for cmd in &commands {
                    ui.push_id(&cmd.id, |ui| {
                        render_command_row(ui, cmd, &mut edit_cmd, &mut remove_id);
                    });
                }
            });

        if let Some(id) = remove_id {
            config.remove_by_id(&id);
            dirty = true;
        }
        if let Some(cmd) = edit_cmd {
            settings.editing = Some(cmd);
            settings.creating_new = false;
        }
    }

    dirty
}

fn render_command_row(
    ui: &mut egui::Ui,
    cmd: &QuickCommand,
    edit_cmd: &mut Option<QuickCommand>,
    remove_id: &mut Option<String>,
) {
    let row_frame = egui::Frame::none()
        .fill(Color32::from_gray(28))
        .stroke(Stroke::new(1.0, Color32::from_gray(50)))
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(10.0, 6.0));

    row_frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            // Left side: name + info
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&cmd.name)
                        .monospace()
                        .size(13.0)
                        .color(Color32::from_gray(220))
                        .strong(),
                );
                ui.horizontal(|ui| {
                    // Tag badge
                    let tag_frame = egui::Frame::none()
                        .fill(Color32::from_rgb(50, 60, 80))
                        .rounding(egui::Rounding::same(3.0))
                        .inner_margin(egui::Margin::symmetric(5.0, 1.0));
                    tag_frame.show(ui, |ui| {
                        ui.label(
                            RichText::new(&cmd.tag)
                                .monospace()
                                .size(10.0)
                                .color(Color32::from_rgb(140, 180, 255)),
                        );
                    });

                    ui.label(
                        RichText::new(format!("$ {}", truncate_str(&cmd.command, 40)))
                            .monospace()
                            .size(11.0)
                            .color(Color32::from_gray(140)),
                    );

                    if cmd.auto_execute {
                        ui.label(
                            RichText::new("[auto]")
                                .monospace()
                                .size(10.0)
                                .color(Color32::from_rgb(100, 200, 100)),
                        );
                    }

                    if !cmd.keybinding.is_empty() {
                        ui.label(
                            RichText::new(format!("[{}]", cmd.keybinding.display()))
                                .monospace()
                                .size(10.0)
                                .color(Color32::from_rgb(200, 180, 100)),
                        );
                    }
                });
            });

            // Right side: edit / delete buttons
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("🗑")
                                .size(13.0)
                                .color(Color32::from_rgb(220, 80, 80)),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Delete")
                    .clicked()
                {
                    *remove_id = Some(cmd.id.clone());
                }

                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("✏")
                                .size(13.0)
                                .color(Color32::from_gray(180)),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Edit")
                    .clicked()
                {
                    *edit_cmd = Some(cmd.clone());
                }
            });
        });
    });
    ui.add_space(3.0);
}

// ---------------------------------------------------------------------------
// Edit / create form
// ---------------------------------------------------------------------------

fn render_edit_form(
    ui: &mut egui::Ui,
    settings: &mut SettingsState,
    config: &mut QuickCommandConfig,
) -> bool {
    let mut dirty = false;
    let title = if settings.creating_new {
        "New Quick Command"
    } else {
        "Edit Quick Command"
    };
    ui.label(
        RichText::new(title)
            .monospace()
            .size(14.0)
            .color(Color32::from_gray(220))
            .strong(),
    );
    ui.add_space(6.0);

    let cmd = settings.editing.as_mut().unwrap();

    egui::Grid::new("quickcmd_edit_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            // Name
            ui.label(RichText::new("Name").monospace().size(12.0).color(Color32::from_gray(160)));
            ui.add(
                egui::TextEdit::singleline(&mut cmd.name)
                    .desired_width(300.0)
                    .hint_text("e.g., List Files"),
            );
            ui.end_row();

            // Command
            ui.label(
                RichText::new("Command").monospace().size(12.0).color(Color32::from_gray(160)),
            );
            ui.add(
                egui::TextEdit::singleline(&mut cmd.command)
                    .desired_width(300.0)
                    .font(egui::FontId::monospace(12.0))
                    .hint_text("e.g., ls -la"),
            );
            ui.end_row();

            // Tag
            ui.label(RichText::new("Tag").monospace().size(12.0).color(Color32::from_gray(160)));
            ui.add(
                egui::TextEdit::singleline(&mut cmd.tag)
                    .desired_width(200.0)
                    .hint_text("e.g., git, docker, default"),
            );
            ui.end_row();

            // Auto execute toggle
            ui.label(
                RichText::new("Auto Execute")
                    .monospace()
                    .size(12.0)
                    .color(Color32::from_gray(160)),
            );
            ui.horizontal(|ui| {
                ui.checkbox(&mut cmd.auto_execute, "");
                ui.label(
                    RichText::new(if cmd.auto_execute {
                        "Send + Enter (auto run)"
                    } else {
                        "Paste only (manual run)"
                    })
                    .monospace()
                    .size(11.0)
                    .color(Color32::from_gray(130)),
                );
            });
            ui.end_row();

            // Keybinding
            ui.label(
                RichText::new("Shortcut Key")
                    .monospace()
                    .size(12.0)
                    .color(Color32::from_gray(160)),
            );
            ui.horizontal(|ui| {
                if settings.recording_keybinding {
                    ui.label(
                        RichText::new("Press key combo...")
                            .monospace()
                            .size(12.0)
                            .color(Color32::from_rgb(255, 200, 80))
                            .strong(),
                    );
                    // Capture keyboard
                    let events = ui.input(|i| i.events.clone());
                    for ev in &events {
                        if let egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } = ev
                        {
                            if matches!(key, egui::Key::Escape) {
                                settings.recording_keybinding = false;
                                break;
                            }

                            let key_name = format!("{:?}", key);
                            cmd.keybinding = KeyBinding {
                                ctrl: modifiers.ctrl,
                                alt: modifiers.alt,
                                shift: modifiers.shift,
                                key: key_name,
                            };
                            settings.recording_keybinding = false;
                            break;
                        }
                    }
                    if ui
                        .add(egui::Button::new(
                            RichText::new("Cancel").monospace().size(11.0),
                        ))
                        .clicked()
                    {
                        settings.recording_keybinding = false;
                    }
                } else {
                    let display = if cmd.keybinding.is_empty() {
                        "None".to_string()
                    } else {
                        cmd.keybinding.display()
                    };
                    let kb_frame = egui::Frame::none()
                        .fill(Color32::from_gray(35))
                        .stroke(Stroke::new(1.0, Color32::from_gray(60)))
                        .rounding(egui::Rounding::same(3.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 3.0));
                    kb_frame.show(ui, |ui| {
                        ui.label(
                            RichText::new(&display)
                                .monospace()
                                .size(12.0)
                                .color(Color32::from_gray(190)),
                        );
                    });
                    if ui
                        .add(egui::Button::new(
                            RichText::new("Record").monospace().size(11.0),
                        ))
                        .clicked()
                    {
                        settings.recording_keybinding = true;
                    }
                    if !cmd.keybinding.is_empty()
                        && ui
                            .add(egui::Button::new(
                                RichText::new("Clear").monospace().size(11.0),
                            ))
                            .clicked()
                    {
                        cmd.keybinding = KeyBinding::default();
                    }
                }
            });
            ui.end_row();
        });

    ui.add_space(12.0);

    // Snapshot validation values before dropping the mutable borrow on settings.editing
    let can_save = {
        let cmd = settings.editing.as_ref().unwrap();
        !cmd.name.trim().is_empty() && !cmd.command.trim().is_empty()
    };

    // Action buttons
    ui.horizontal(|ui| {
        let save_btn = egui::Button::new(
            RichText::new("Save")
                .monospace()
                .size(12.0)
                .color(Color32::WHITE),
        )
        .fill(if can_save {
            Color32::from_rgb(45, 125, 235)
        } else {
            Color32::from_gray(60)
        })
        .stroke(Stroke::new(
            1.0,
            if can_save {
                Color32::from_rgb(90, 160, 255)
            } else {
                Color32::from_gray(80)
            },
        ));

        let save_resp = ui.add_enabled(can_save, save_btn);
        if save_resp.clicked() {
            let edited = settings.editing.take().unwrap();
            if settings.creating_new {
                config.commands.push(edited);
            } else {
                // Update existing
                if let Some(existing) = config.commands.iter_mut().find(|c| c.id == edited.id) {
                    *existing = edited;
                }
            }
            settings.creating_new = false;
            dirty = true;
        }

        if ui
            .add(egui::Button::new(
                RichText::new("Cancel").monospace().size(12.0),
            ))
            .clicked()
        {
            settings.editing = None;
            settings.creating_new = false;
        }
    });

    dirty
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}
