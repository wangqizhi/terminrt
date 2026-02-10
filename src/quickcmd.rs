use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A shortcut key combination for a quick command.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key character (e.g. "1", "a", "F5").
    pub key: String,
}

impl KeyBinding {
    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    pub fn display(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

/// A single quick command entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuickCommand {
    /// Unique identifier.
    pub id: String,
    /// Display name shown on the button.
    pub name: String,
    /// The command string to send to the terminal.
    pub command: String,
    /// If true, append Enter (auto‑execute). Otherwise just paste into prompt.
    pub auto_execute: bool,
    /// Tag(s) used for grouping display in the right panel.
    pub tag: String,
    /// Optional keyboard shortcut.
    pub keybinding: KeyBinding,
}

impl QuickCommand {
    pub fn new_empty() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            command: String::new(),
            auto_execute: true,
            tag: "default".to_string(),
            keybinding: KeyBinding::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Config persistence
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuickCommandConfig {
    pub commands: Vec<QuickCommand>,
}

impl QuickCommandConfig {
    /// Return ordered, deduplicated tag list.
    pub fn tags(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .commands
            .iter()
            .map(|c| c.tag.clone())
            .filter(|t| !t.is_empty())
            .collect();
        set.into_iter().collect()
    }

    pub fn commands_by_tag(&self, tag: &str) -> Vec<&QuickCommand> {
        self.commands.iter().filter(|c| c.tag == tag).collect()
    }

    pub fn remove_by_id(&mut self, id: &str) {
        self.commands.retain(|c| c.id != id);
    }

    pub fn find_by_keybinding(&self, kb: &KeyBinding) -> Option<&QuickCommand> {
        if kb.is_empty() {
            return None;
        }
        self.commands.iter().find(|c| c.keybinding == *kb)
    }
}

fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("terminrt").join("quickcmds.json")
}

pub fn load_config() -> QuickCommandConfig {
    let path = config_path();
    if !path.exists() {
        return QuickCommandConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => QuickCommandConfig::default(),
    }
}

pub fn save_config(config: &QuickCommandConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(&path, json);
    }
}
