//! Persistent settings (stored as JSON in the platform config dir).

use crate::db::ConnectionConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneralSettings {
    /// System font family used for the interface; empty = built-in.
    pub ui_font: String,
    /// System font family used for the editor and terminal; empty = built-in mono.
    pub editor_font: String,
    pub font_size: f32,
    /// Interface zoom, stored in percent (steps of 10).
    pub ui_scale_pct: u32,
    pub theme: String,
    pub indent_size: usize,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            ui_font: String::new(),
            editor_font: String::new(),
            font_size: 13.0,
            ui_scale_pct: 100,
            theme: "dark".into(),
            indent_size: 4,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PythonInterpreter {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CodingSettings {
    pub interpreters: Vec<PythonInterpreter>,
    pub active_interpreter: Option<usize>,
    pub connections: Vec<ConnectionConfig>,
    pub active_connection: Option<String>,
    pub sql_lint: bool,
    pub yaml_lint: bool,
    /// Row limit applied by "Quick inspect".
    pub inspect_limit: usize,
    /// Rows kept in the result grid for regular queries.
    pub max_result_rows: usize,
    pub query_timeout_secs: u64,
}

impl Default for CodingSettings {
    fn default() -> Self {
        Self {
            interpreters: Vec::new(),
            active_interpreter: None,
            connections: Vec::new(),
            active_connection: None,
            sql_lint: true,
            yaml_lint: true,
            inspect_limit: 500,
            max_result_rows: 10_000,
            query_timeout_secs: 300,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TerminalSettings {
    pub shell: String,
    pub font_size: f32,
    pub scrollback: usize,
    /// Extra environment variables, `KEY=VALUE` per entry.
    pub env: Vec<String>,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
            font_size: 13.0,
            scrollback: 5000,
            env: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OtherSettings {
    pub scratches_dir: PathBuf,
    pub restore_session: bool,
    /// Command launched by the "Claude Code" action.
    pub claude_command: String,
    pub show_hidden_files: bool,
    pub autosave_on_focus_loss: bool,
}

impl Default for OtherSettings {
    fn default() -> Self {
        Self {
            scratches_dir: dirs::home_dir().unwrap_or_default().join("LudoScratches"),
            restore_session: true,
            claude_command: "claude".into(),
            show_hidden_files: false,
            autosave_on_focus_loss: true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub general: GeneralSettings,
    pub coding: CodingSettings,
    pub terminal: TerminalSettings,
    pub other: OtherSettings,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("ludo")
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        save_json(&Self::path(), self)
    }

    pub fn interpreter(&self) -> Option<&PythonInterpreter> {
        self.coding.active_interpreter.and_then(|i| self.coding.interpreters.get(i))
    }

    pub fn indent(&self) -> usize {
        self.general.indent_size.clamp(1, 8)
    }
}

pub fn save_json<T: Serialize>(path: &std::path::Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)
}

/// Window/tabs state restored on the next launch.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Session {
    pub roots: Vec<PathBuf>,
    pub open_files: Vec<PathBuf>,
    pub active_tab: usize,
    pub terminal_names: Vec<String>,
}

impl Session {
    fn path() -> PathBuf {
        config_dir().join("session.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        save_json(&Self::path(), self)
    }
}
