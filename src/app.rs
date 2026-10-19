//! Application state and top-level layout.

use crate::db::worker::{Request, Worker};
use crate::db::{self, ConnectionConfig};
use crate::editor::{Document, OpenOutcome, Tabs};
use crate::fonts::{self, SystemFonts};
use crate::git::GitCache;
use crate::languages::{self, Runner};
use crate::lint::{self, LintContext};
use crate::settings::{Session, Settings};
use crate::terminal::{SpawnSpec, TerminalSession};
use crate::theme::{self, Palette};
use crate::ui::bottom::{BottomTab, Terminals};
use crate::ui::db_panel::DbState;
use crate::ui::files::{FileDialog, Workspace};
use crate::ui::python::PythonState;
use crate::ui::settings_window::SettingsTab;
use egui::{Key, KeyboardShortcut, Modifiers};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftTool {
    Explorer,
    Python,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub enum Confirm {
    CloseDirty(usize),
    DeletePath(PathBuf),
    DeleteConnection(String),
}

#[derive(Clone, PartialEq)]
struct AppliedLook {
    ui_font: String,
    editor_font: String,
    font_size: f32,
    scale: u32,
    theme: String,
}

pub struct App {
    pub settings: Settings,
    saved_settings: Settings,
    pub palette: Palette,
    pub theme_id: &'static str,
    pub sys_fonts: SystemFonts,
    applied: Option<AppliedLook>,
    pub tabs: Tabs,
    pub workspace: Workspace,
    pub left_tool: LeftTool,
    pub left_open: bool,
    pub right_open: bool,
    pub bottom_open: bool,
    pub bottom_tab: BottomTab,
    pub db: DbState,
    pub terms: Terminals,
    pub python: PythonState,
    pub toast: Option<Toast>,
    pub file_dialog: Option<FileDialog>,
    pub confirm: Option<Confirm>,
    pub settings_open: bool,
    pub settings_tab: SettingsTab,
    pub git: GitCache,
    pub md_cache: egui_commonmark::CommonMarkCache,
    was_focused: bool,
    saved_session: Option<Session>,
    pub pending_copy: Option<String>,
    pub find: Option<crate::ui::find::FindState>,
    pub compile: Option<crate::ui::dbt_compile::CompileDialog>,
    pub toast_text: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let _ = std::fs::create_dir_all(&settings.other.scratches_dir);
        let mut app = Self {
            saved_settings: settings.clone(),
            palette: theme::palette(&settings.general.theme),
            theme_id: "dark",
            settings,
            sys_fonts: SystemFonts::default(),
            applied: None,
            tabs: Tabs::default(),
            workspace: Workspace::default(),
            left_tool: LeftTool::Explorer,
            left_open: true,
            right_open: true,
            bottom_open: true,
            bottom_tab: BottomTab::Terminal,
            db: DbState::new(Worker::new(cc.egui_ctx.clone())),
            terms: Terminals::default(),
            python: PythonState::default(),
            toast: None,
            file_dialog: None,
            confirm: None,
            settings_open: false,
            settings_tab: SettingsTab::General,
            git: GitCache::default(),
            md_cache: Default::default(),
            was_focused: true,
            saved_session: None,
            pending_copy: None,
            find: None,
            compile: None,
            toast_text: None,
        };
        app.migrate_passwords();
        if app.settings.other.restore_session {
            let session = Session::load();
            app.workspace.roots = session.roots.into_iter().filter(|p| p.is_dir()).collect();
            for f in session.open_files.iter().filter(|p| p.is_file()) {
                let _ = app.tabs.open_path(f);
            }
            app.tabs.activate(session.active_tab.min(app.tabs.docs.len().saturating_sub(1)));
            for name in session.terminal_names {
                app.new_terminal(Some(name), None, None, &cc.egui_ctx);
            }
        }
        if let Some(dir) = std::env::args().nth(1).map(PathBuf::from) {
            if dir.is_dir() {
                app.workspace.add_root(dir);
            } else if dir.is_file() {
                let _ = app.tabs.open_path(&dir);
            }
        }
        app
    }

    /// Move plain-text passwords (older settings files) into the Keychain. The
    /// settings file is rewritten without them on the next persist.
    fn migrate_passwords(&mut self) {
        let mut failed = Vec::new();
        for c in &mut self.settings.coding.connections {
            if c.password.is_empty() {
                continue;
            }
            match crate::secrets::set(&c.id, &c.password) {
                Ok(()) => {
                    c.password.clear();
                    c.keychain_password = true;
                }
                Err(e) => failed.push(format!("{}: {e}", c.display_name())),
            }
        }
        if !failed.is_empty() {
            self.error(format!(
                "Some passwords could not be moved to the Keychain and stay in settings.json — {}",
                failed.join("; ")
            ));
        }
    }

    // ----- helpers used by the panels -----

    pub fn notify(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), error: false, at: Instant::now() });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), error: true, at: Instant::now() });
    }

    pub fn open_file(&mut self, path: &Path) {
        match self.tabs.open_path(path) {
            Ok(OpenOutcome::Opened) => {}
            Ok(OpenOutcome::Evicted(name)) => self.notify(format!("Tab limit reached — closed '{name}'")),
            Err(e) => self.error(e),
        }
    }

    pub fn open_document(&mut self, doc: Document) {
        match self.tabs.push(doc) {
            Ok(OpenOutcome::Opened) => {}
            Ok(OpenOutcome::Evicted(name)) => self.notify(format!("Tab limit reached — closed '{name}'")),
            Err(e) => self.error(e),
        }
    }

    pub fn active_connection(&self) -> Option<&ConnectionConfig> {
        let id = self.settings.coding.active_connection.as_deref()?;
        self.settings.coding.connections.iter().find(|c| c.id == id)
    }

    pub fn connection(&self, id: &str) -> Option<&ConnectionConfig> {
        self.settings.coding.connections.iter().find(|c| c.id == id)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.settings.coding.query_timeout_secs.max(1))
    }

    pub fn db_request(&mut self, conn_id: &str, req: Request) {
        if let Some(cfg) = self.connection(conn_id).cloned() {
            self.db.submit(&cfg, self.timeout(), req);
        }
    }

    /// The workspace root that contains `path`, if any.
    pub fn root_for(&self, path: &Path) -> Option<PathBuf> {
        let scratches = &self.settings.other.scratches_dir;
        self.workspace
            .roots
            .iter()
            .chain(std::iter::once(scratches))
            .filter(|r| path.starts_with(r))
            .max_by_key(|r| r.components().count())
            .cloned()
    }

    pub fn context_dir(&self) -> Option<PathBuf> {
        self.tabs
            .current()
            .and_then(|d| d.path.as_deref())
            .and_then(|p| self.root_for(p))
            .or_else(|| self.workspace.roots.first().cloned())
    }

    pub fn new_terminal(
        &mut self,
        name: Option<String>,
        program: Option<(PathBuf, Vec<String>)>,
        cwd: Option<PathBuf>,
        ctx: &egui::Context,
    ) {
        let n = self.terms.sessions.len() + 1;
        let spec = SpawnSpec {
            name: name.unwrap_or_else(|| if n == 1 { "Local".into() } else { format!("Local ({n})") }),
            program,
            cwd: cwd.or_else(|| self.context_dir()),
            shell: self.settings.terminal.shell.clone(),
            env: self.settings.terminal.env.clone(),
            scrollback: self.settings.terminal.scrollback,
        };
        match TerminalSession::spawn(spec, ctx.clone()) {
            Ok(t) => {
                self.terms.sessions.push(t);
                self.terms.active = self.terms.sessions.len() - 1;
                self.terms.focus_next_frame = true;
                self.bottom_open = true;
                self.bottom_tab = BottomTab::Terminal;
            }
            Err(e) => self.error(format!("Cannot start terminal: {e}")),
        }
    }

    /// Start Claude Code in a terminal; `cwd` defaults to the current project root.
    pub fn launch_claude(&mut self, prompt: Option<String>, cwd: Option<PathBuf>, ctx: &egui::Context) {
        let mut cmd = self.settings.other.claude_command.clone();
        if let Some(p) = prompt {
            cmd.push_str(&format!(" '{}'", p.replace('\'', "'\\''")));
        }
        let cwd = cwd.or_else(|| self.context_dir());
        let name = match cwd.as_ref().and_then(|d| d.file_name()) {
            Some(n) => format!("Claude · {}", n.to_string_lossy()),
            None => "Claude".into(),
        };
        let shell = PathBuf::from(&self.settings.terminal.shell);
        self.new_terminal(Some(name), Some((shell, vec!["-lic".into(), cmd])), cwd, ctx);
    }

    /// Run the current document: SQL against the DB, Python in a terminal.
    pub fn run_current(&mut self, whole_script: bool, ctx: &egui::Context) {
        let Some(doc) = self.tabs.current_mut() else { return };
        let lang = languages::get(doc.lang);
        match lang.runner {
            Runner::Database => {
                let (statements, label) = if whole_script {
                    (split_script(&doc.text), "script")
                } else if doc.sel.0 != doc.sel.1 {
                    (split_script(&doc.selected_text()), "selection")
                } else {
                    let caret = doc.sel.1;
                    (vec![crate::editor::ops::statement_at(&doc.text, caret)], "statement")
                };
                let statements: Vec<String> = statements.into_iter().filter(|s| !s.trim().is_empty()).collect();
                if statements.is_empty() {
                    self.error(format!("Nothing to run in {label}"));
                    return;
                }
                self.run_sql(statements);
            }
            Runner::Python => {
                let Some(path) = doc.path.clone() else { return };
                if doc.is_dirty() {
                    let _ = doc.save();
                }
                let Some(interp) = self.settings.interpreter().cloned() else {
                    self.error("Select a Python interpreter first (Python panel or Settings → Coding)");
                    self.left_tool = LeftTool::Python;
                    self.left_open = true;
                    return;
                };
                let cwd = self.root_for(&path).unwrap_or_else(|| TerminalSession::cwd_hint(&path));
                let name = format!("▶ {}", path.file_name().unwrap_or_default().to_string_lossy());
                // Through a login shell so the script sees the user's PATH (GUI apps on
                // macOS start with a minimal one); `$0`/`$@` avoid any quoting issues.
                let shell = PathBuf::from(&self.settings.terminal.shell);
                let args = vec![
                    "-l".into(),
                    "-c".into(),
                    "exec \"$0\" \"$@\"".into(),
                    interp.path.to_string_lossy().into_owned(),
                    path.to_string_lossy().into_owned(),
                ];
                self.new_terminal(Some(name), Some((shell, args)), Some(cwd), ctx);
            }
            Runner::Shell | Runner::None => self.notify(format!("{} files are not runnable", lang.name)),
        }
    }

    pub fn run_sql(&mut self, statements: Vec<String>) {
        let Some(cfg) = self.active_connection().cloned() else {
            self.error("No active database connection — add or select one in the Databases panel");
            self.right_open = true;
            return;
        };
        if self.db.running.is_some() {
            self.error("A query is already running");
            return;
        }
        let max_rows = self.settings.coding.max_result_rows;
        self.db.start_script(&cfg, self.timeout(), statements, max_rows);
        self.bottom_open = true;
        self.bottom_tab = BottomTab::Results;
    }

    pub fn new_scratch(&mut self, dir: &Path, ext: &str) {
        let _ = std::fs::create_dir_all(dir);
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let mut path = dir.join(format!("scratch_{stamp}.{ext}"));
        let mut i = 1;
        while path.exists() {
            path = dir.join(format!("scratch_{stamp}_{i}.{ext}"));
            i += 1;
        }
        match std::fs::write(&path, "") {
            Ok(()) => {
                self.workspace.invalidate(dir);
                self.open_file(&path);
            }
            Err(e) => self.error(format!("Cannot create {}: {e}", path.display())),
        }
    }

    pub fn save_current(&mut self) {
        if let Some(doc) = self.tabs.current_mut() {
            if let Err(e) = doc.save() {
                let msg = format!("Save failed: {e}");
                self.error(msg);
            }
        }
    }

    pub fn request_close(&mut self, i: usize) {
        if self.tabs.docs.get(i).is_some_and(|d| d.is_dirty()) {
            self.confirm = Some(Confirm::CloseDirty(i));
        } else {
            self.tabs.close(i);
        }
    }

    fn save_session(&mut self) {
        let session = self.current_session();
        if self.saved_session.as_ref() != Some(&session) {
            let _ = session.save();
            self.saved_session = Some(session);
        }
    }

    fn current_session(&self) -> Session {
        Session {
            roots: self.workspace.roots.clone(),
            open_files: self.tabs.docs.iter().filter_map(|d| d.path.clone()).collect(),
            active_tab: self.tabs.active,
            terminal_names: self
                .terms
                .sessions
                .iter()
                .filter(|t| t.exited.is_none() && !t.name.starts_with('▶') && !t.name.starts_with("Claude"))
                .map(|t| t.name.clone())
                .collect(),
        }
    }

    // ----- per-frame logic -----

    fn apply_look(&mut self, ctx: &egui::Context) {
        let g = &self.settings.general;
        let look = AppliedLook {
            ui_font: g.ui_font.clone(),
            editor_font: g.editor_font.clone(),
            font_size: g.font_size.clamp(8.0, 32.0),
            scale: g.ui_scale_pct.clamp(50, 300),
            theme: g.theme.clone(),
        };
        let prev = self.applied.clone();
        if prev.as_ref() == Some(&look) {
            return;
        }
        if prev.as_ref().is_none_or(|p| p.ui_font != look.ui_font || p.editor_font != look.editor_font) {
            ctx.set_fonts(fonts::build(&mut self.sys_fonts, &look.ui_font, &look.editor_font));
        }
        let t = theme::THEMES.iter().find(|t| t.id == look.theme).unwrap_or(&theme::THEMES[0]);
        self.theme_id = t.id;
        self.palette = t.palette;
        let visuals = theme::visuals(&self.palette);
        let size = look.font_size;
        ctx.all_styles_mut(|s| {
            use egui::{FontFamily, FontId, TextStyle};
            s.visuals = visuals.clone();
            s.text_styles = [
                (TextStyle::Small, FontId::new(size - 2.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(size, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(size, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(size + 5.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(size, FontFamily::Monospace)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(6.0, 2.0);
            s.spacing.interact_size.y = size + 9.0;
            s.spacing.scroll.floating = true;
            s.interaction.selectable_labels = false;
        });
        ctx.set_zoom_factor(look.scale as f32 / 100.0);
        self.applied = Some(look);
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = Modifiers::COMMAND;
        let sc = |m, k| KeyboardShortcut::new(m, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));

        if pressed(sc(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab)) {
            self.tabs.cycle(false);
        }
        if pressed(sc(Modifiers::CTRL, Key::Tab)) {
            self.tabs.cycle(true);
        }
        if pressed(sc(cmd | Modifiers::SHIFT, Key::OpenBracket)) {
            self.tabs.cycle(false);
        }
        if pressed(sc(cmd | Modifiers::SHIFT, Key::CloseBracket)) {
            self.tabs.cycle(true);
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
            Key::Num0,
        ];
        for (i, k) in digits.into_iter().enumerate() {
            if pressed(sc(cmd, k)) {
                self.tabs.activate(i);
            }
        }
        if pressed(sc(cmd, Key::F)) {
            self.open_find();
        }
        if pressed(sc(cmd, Key::S)) {
            self.save_current();
        }
        if pressed(sc(cmd, Key::W)) {
            self.request_close(self.tabs.active);
        }
        if pressed(sc(cmd | Modifiers::SHIFT, Key::Enter)) {
            self.run_current(true, ctx);
        }
        if pressed(sc(cmd, Key::Enter)) {
            self.run_current(false, ctx);
        }
        if pressed(sc(cmd, Key::Comma)) {
            self.settings_open = !self.settings_open;
        }
        if pressed(sc(cmd, Key::B)) {
            self.left_open = !self.left_open;
        }
        if pressed(sc(cmd, Key::J)) {
            self.bottom_open = !self.bottom_open;
        }
        if pressed(sc(cmd | Modifiers::SHIFT, Key::D)) {
            self.right_open = !self.right_open;
        }
        if pressed(sc(cmd | Modifiers::SHIFT, Key::Backtick))
            || pressed(sc(Modifiers::CTRL | Modifiers::SHIFT, Key::Backtick))
        {
            self.new_terminal(None, None, None, ctx);
        }
        if pressed(sc(cmd, Key::N)) {
            let dir = self.settings.other.scratches_dir.clone();
            self.new_scratch(&dir, "sql");
        }
        if pressed(sc(cmd, Key::Plus)) || pressed(sc(cmd, Key::Equals)) {
            self.settings.general.ui_scale_pct = (self.settings.general.ui_scale_pct + 10).min(300);
        }
        if pressed(sc(cmd, Key::Minus)) {
            self.settings.general.ui_scale_pct = self.settings.general.ui_scale_pct.saturating_sub(10).max(50);
        }
    }

    fn lint_tick(&mut self, ctx: &egui::Context) {
        let dialect = self.active_connection().map(|c| c.kind).unwrap_or(db::DriverKind::ClickHouse).sql_dialect();
        let lc = LintContext {
            sql_dialect: dialect.as_ref(),
            sql: self.settings.coding.sql_lint,
            yaml: self.settings.coding.yaml_lint,
        };
        let debounce = Duration::from_millis(350);
        for doc in &mut self.tabs.docs {
            // Server-generated DDL views use syntax the generic parser may not know.
            if doc.linted_version == Some(doc.version) || doc.path.is_none() {
                continue;
            }
            if doc.last_edit.elapsed() < debounce && doc.linted_version.is_some() {
                ctx.request_repaint_after(debounce);
                continue;
            }
            doc.diagnostics = lint::lint(doc.lang, &doc.text, &lc);
            doc.linted_version = Some(doc.version);
        }
    }

    fn focus_tick(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused && !self.was_focused {
            for doc in &mut self.tabs.docs {
                doc.reload_if_changed();
            }
            self.workspace.invalidate_all();
        }
        if !focused && self.was_focused && self.settings.other.autosave_on_focus_loss {
            for doc in &mut self.tabs.docs {
                if doc.is_dirty() {
                    let _ = doc.save();
                }
            }
        }
        self.was_focused = focused;
    }

    fn persist_settings(&mut self) {
        if self.settings != self.saved_settings {
            if let Err(e) = self.settings.save() {
                self.error(format!("Cannot save settings: {e}"));
            }
            self.saved_settings = self.settings.clone();
        }
    }
}

/// Split a script into statements (respecting quotes and `--` comments).
pub fn split_script(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        match quote {
            Some(q) => {
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                    }
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '-' if chars.peek() == Some(&'-') => {
                    for n in chars.by_ref() {
                        cur.push(n);
                        if n == '\n' {
                            break;
                        }
                    }
                }
                ';' => {
                    cur.pop();
                    out.push(std::mem::take(&mut cur));
                }
                _ => {}
            },
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| s.lines().any(|l| !l.trim().is_empty() && !l.trim().starts_with("--")))
        .collect()
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_look(ctx);
        self.global_shortcuts(ctx);
        self.poll_db();
        self.lint_tick(ctx);
        self.focus_tick(ctx);
        self.persist_settings();
        self.save_session();
        if let Some(t) = &self.toast {
            if t.at.elapsed() > Duration::from_secs(if t.error { 8 } else { 4 }) {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        // Keep the status-bar clock ticking.
        ctx.request_repaint_after(Duration::from_secs(15));
        if self.db.running.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        let title = match self.tabs.current() {
            Some(d) => format!("{}{} — Ludo", d.name(), if d.is_dirty() { " •" } else { "" }),
            None => "Ludo".into(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.status_bar(ui);
        self.activity_bar_left(ui);
        self.activity_bar_right(ui);
        if self.left_open {
            self.left_panel(ui);
        }
        if self.right_open {
            self.db_panel(ui);
        }
        if self.bottom_open {
            self.bottom_panel(ui);
        }
        self.editor_area(ui);
        self.dialogs(&ctx);
        if self.settings_open {
            self.settings_window(&ctx);
        }
        self.toast_overlay(&ctx);
        if let Some(text) = self.toast_text.take() {
            self.notify(text);
        }
        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_session();
        self.persist_settings();
    }

    fn on_exit(&mut self) {
        for doc in &mut self.tabs.docs {
            if doc.is_dirty() && self.settings.other.autosave_on_focus_loss {
                let _ = doc.save();
            }
        }
        self.save_session();
        self.persist_settings();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_scripts() {
        let s = super::split_script("select 1;\n-- comment;\nselect ';';\n\n-- trailing\n");
        assert_eq!(s, vec!["select 1".to_owned(), "-- comment;\nselect ';'".to_owned()]);
    }
}
