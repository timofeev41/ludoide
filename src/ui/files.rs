//! Explorer: workspace roots plus the always-present Scratches directory.

use super::{dim, panel_frame, section_header, small_icon_button};
use crate::app::{App, Confirm, LeftTool};
use crate::fonts::icon;
use crate::languages::{self, LangId};
use egui::{CollapsingHeader, RichText};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Default)]
pub struct Workspace {
    pub roots: Vec<PathBuf>,
    cache: HashMap<PathBuf, Vec<Entry>>,
    pub filter: String,
    /// Bumped by "Collapse all": fresh header ids start closed.
    collapse_gen: u64,
}

const IGNORED: &[&str] = &[".git", "__pycache__", ".DS_Store", ".idea", ".mypy_cache", ".pytest_cache", ".ruff_cache"];

impl Workspace {
    pub fn add_root(&mut self, dir: PathBuf) {
        if !self.roots.contains(&dir) {
            self.roots.push(dir);
        }
    }

    pub fn invalidate(&mut self, dir: &Path) {
        self.cache.remove(dir);
    }

    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }

    fn children(&mut self, dir: &Path, show_hidden: bool) -> Vec<Entry> {
        if let Some(c) = self.cache.get(dir) {
            return c.clone();
        }
        let mut entries: Vec<Entry> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| {
                        let path = e.path();
                        Entry { name: e.file_name().to_string_lossy().into_owned(), is_dir: path.is_dir(), path }
                    })
                    .filter(|e| !IGNORED.contains(&e.name.as_str()))
                    .filter(|e| show_hidden || !e.name.starts_with('.') || e.name == ".env")
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.cache.insert(dir.to_path_buf(), entries.clone());
        entries
    }
}

pub fn file_icon(path: &Path) -> &'static str {
    match languages::detect(path) {
        LangId::Sql => icon::FILE_SQL,
        LangId::Python => icon::FILE_PY,
        LangId::Markdown => icon::FILE_MD,
        LangId::Yaml | LangId::Toml => icon::FILE_INI,
        LangId::Shell => icon::TERMINAL,
        LangId::Json => icon::BRACKETS_CURLY,
        LangId::Plain => icon::FILE_TEXT,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    NewFile,
    NewDir,
    Rename,
}

pub struct FileDialog {
    pub kind: DialogKind,
    /// Parent directory for new entries; the entry itself for rename.
    pub base: PathBuf,
    pub name: String,
    pub focus: bool,
}

enum Action {
    Open(PathBuf),
    Dialog(DialogKind, PathBuf),
    Delete(PathBuf),
    Reveal(PathBuf),
    CopyPath(PathBuf),
    Terminal(PathBuf),
    RemoveRoot(PathBuf),
    NewScratch(PathBuf, &'static str),
    Claude(PathBuf),
    Refresh(PathBuf),
}

impl App {
    pub fn left_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::Panel::left("left_panel")
            .resizable(true)
            .default_size(260.0)
            .size_range(160.0..=600.0)
            .frame(panel_frame(p.panel, 8))
            .show(ui, |ui| match self.left_tool {
                LeftTool::Explorer => self.explorer(ui),
                LeftTool::Python => self.python_panel(ui),
            });
    }

    fn explorer(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            section_header(ui, "Explorer", &p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if small_icon_button(ui, icon::ARROWS_IN_LINE_VERTICAL, "Collapse all", &p).clicked() {
                    self.workspace.collapse_gen += 1;
                }
                if small_icon_button(ui, icon::ARROWS_CLOCKWISE, "Refresh", &p).clicked() {
                    self.workspace.invalidate_all();
                }
                if small_icon_button(ui, icon::FOLDER_PLUS, "Add folder to workspace", &p).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.workspace.add_root(dir);
                    }
                }
                if small_icon_button(ui, icon::FOLDER_OPEN, "Open folder (replaces workspace)", &p).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.workspace.roots = vec![dir];
                        self.workspace.invalidate_all();
                    }
                }
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.workspace.filter)
                .hint_text(format!("{} Filter files", icon::MAGNIFYING_GLASS))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let show_hidden = self.settings.other.show_hidden_files;
        let filter = self.workspace.filter.to_lowercase();
        let scratches = self.settings.other.scratches_dir.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let roots = self.workspace.roots.clone();
            if roots.is_empty() {
                ui.label(dim("No folders open.", &p));
                if ui.button(format!("{} Open folder…", icon::FOLDER_OPEN)).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.workspace.add_root(dir);
                    }
                }
            }
            for root in &roots {
                let name =
                    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
                let header = CollapsingHeader::new(RichText::new(format!("{} {name}", icon::FOLDER)).strong())
                    .id_salt(("root", root, self.workspace.collapse_gen))
                    .default_open(self.workspace.collapse_gen == 0)
                    .show(ui, |ui| self.dir_tree(ui, root, &filter, show_hidden, false, &mut actions));
                header.header_response.on_hover_text(root.display().to_string()).context_menu(|ui| {
                    dir_menu(ui, root, false, &mut actions);
                    ui.separator();
                    if ui.button("Remove from workspace").clicked() {
                        actions.push(Action::RemoveRoot(root.clone()));
                    }
                });
            }
            ui.add_space(6.0);
            // The permanent scratches directory is always listed, whatever project is open.
            let header =
                CollapsingHeader::new(RichText::new(format!("{} Scratches", icon::STAR)).strong().color(p.accent))
                    .id_salt(("scratches_root", self.workspace.collapse_gen))
                    .default_open(self.workspace.collapse_gen == 0)
                    .show(ui, |ui| self.dir_tree(ui, &scratches, &filter, show_hidden, true, &mut actions));
            header
                .header_response
                .on_hover_text(format!(
                    "{}\nSQL runs on the active connection, Python on the selected interpreter",
                    scratches.display()
                ))
                .context_menu(|ui| dir_menu(ui, &scratches, true, &mut actions));
        });
        self.apply_file_actions(actions, ui.ctx());
    }

    fn dir_tree(
        &mut self,
        ui: &mut egui::Ui,
        dir: &Path,
        filter: &str,
        hidden: bool,
        scratch: bool,
        actions: &mut Vec<Action>,
    ) {
        let p = self.palette;
        let entries = self.workspace.children(dir, hidden);
        if entries.is_empty() {
            ui.label(dim("empty", &p).small());
        }
        let active = self.tabs.current().and_then(|d| d.path.clone());
        for e in entries {
            if e.is_dir {
                let r = CollapsingHeader::new(format!("{} {}", icon::FOLDER, e.name))
                    .id_salt(("dir", &e.path, self.workspace.collapse_gen))
                    .default_open(!filter.is_empty())
                    .show(ui, |ui| self.dir_tree(ui, &e.path, filter, hidden, scratch, actions));
                r.header_response.context_menu(|ui| dir_menu(ui, &e.path, scratch, actions));
            } else {
                if !filter.is_empty() && !e.name.to_lowercase().contains(filter) {
                    continue;
                }
                let selected = active.as_deref() == Some(e.path.as_path());
                let text = RichText::new(format!("{} {}", file_icon(&e.path), e.name));
                let r = ui.selectable_label(selected, text);
                if r.clicked() {
                    actions.push(Action::Open(e.path.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        actions.push(Action::Open(e.path.clone()));
                    }
                    if ui.button("Rename…").clicked() {
                        actions.push(Action::Dialog(DialogKind::Rename, e.path.clone()));
                    }
                    if ui.button("Copy path").clicked() {
                        actions.push(Action::CopyPath(e.path.clone()));
                    }
                    if ui.button(reveal_label()).clicked() {
                        actions.push(Action::Reveal(e.path.clone()));
                    }
                    ui.separator();
                    if ui.button(RichText::new("Delete").color(p.error)).clicked() {
                        actions.push(Action::Delete(e.path.clone()));
                    }
                });
            }
        }
    }

    fn apply_file_actions(&mut self, actions: Vec<Action>, ctx: &egui::Context) {
        for a in actions {
            match a {
                Action::Open(path) => self.open_file(&path),
                Action::Dialog(kind, base) => {
                    let name = if kind == DialogKind::Rename {
                        base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    self.file_dialog = Some(FileDialog { kind, base, name, focus: true });
                }
                Action::Delete(path) => self.confirm = Some(Confirm::DeletePath(path)),
                Action::Reveal(path) => reveal(&path),
                Action::CopyPath(path) => ctx.copy_text(path.display().to_string()),
                Action::Terminal(dir) => self.new_terminal(None, None, Some(dir), ctx),
                Action::RemoveRoot(root) => self.workspace.roots.retain(|r| r != &root),
                Action::NewScratch(dir, ext) => self.new_scratch(&dir, ext),
                Action::Claude(dir) => {
                    // Always the project root (or Scratches), even from a nested folder.
                    let root = self.root_for(&dir).unwrap_or(dir);
                    self.launch_claude(None, Some(root), ctx);
                }
                Action::Refresh(dir) => self.workspace.invalidate(&dir),
            }
        }
    }

    pub fn file_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.file_dialog else { return };
        let title = match dlg.kind {
            DialogKind::NewFile => "New file",
            DialogKind::NewDir => "New folder",
            DialogKind::Rename => "Rename",
        };
        let mut submit = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("file_dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(title);
            ui.add_space(6.0);
            let r =
                ui.add(egui::TextEdit::singleline(&mut dlg.name).desired_width(f32::INFINITY).hint_text("name.sql"));
            if dlg.focus {
                r.request_focus();
                dlg.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.file_dialog = None;
            return;
        }
        if !submit {
            return;
        }
        let dlg = self.file_dialog.take().unwrap();
        let name = dlg.name.trim();
        if name.is_empty() || name.contains('/') && dlg.kind == DialogKind::Rename {
            self.error("Invalid name");
            return;
        }
        let result = match dlg.kind {
            DialogKind::NewFile => {
                let path = dlg.base.join(name);
                let r = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|_| std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map(|_| ()));
                self.workspace.invalidate_all();
                r.map(|_| self.open_file(&path))
            }
            DialogKind::NewDir => {
                let r = std::fs::create_dir_all(dlg.base.join(name));
                self.workspace.invalidate(&dlg.base);
                r
            }
            DialogKind::Rename => {
                let to = dlg.base.with_file_name(name);
                let r = std::fs::rename(&dlg.base, &to);
                if r.is_ok() {
                    self.tabs.rename_path(&dlg.base, &to);
                    if let Some(i) = self.workspace.roots.iter().position(|r| r == &dlg.base) {
                        self.workspace.roots[i] = to.clone();
                    }
                }
                self.workspace.invalidate_all();
                r
            }
        };
        if let Err(e) = result {
            self.error(format!("{title} failed: {e}"));
        }
    }
}

fn dir_menu(ui: &mut egui::Ui, dir: &Path, scratch: bool, actions: &mut Vec<Action>) {
    if scratch {
        for (label, ext) in [
            ("New SQL scratch", "sql"),
            ("New Python scratch", "py"),
            ("New shell script", "sh"),
            ("New Markdown note", "md"),
        ] {
            if ui.button(label).clicked() {
                actions.push(Action::NewScratch(dir.to_path_buf(), ext));
            }
        }
        ui.separator();
    }
    if ui.button(format!("{} Open in Claude", icon::SPARKLE)).on_hover_text("Claude Code in the project root").clicked()
    {
        actions.push(Action::Claude(dir.to_path_buf()));
    }
    ui.separator();
    if ui.button("New file…").clicked() {
        actions.push(Action::Dialog(DialogKind::NewFile, dir.to_path_buf()));
    }
    if ui.button("New folder…").clicked() {
        actions.push(Action::Dialog(DialogKind::NewDir, dir.to_path_buf()));
    }
    ui.separator();
    if ui.button("Rename…").clicked() {
        actions.push(Action::Dialog(DialogKind::Rename, dir.to_path_buf()));
    }
    if ui.button("Open in terminal").clicked() {
        actions.push(Action::Terminal(dir.to_path_buf()));
    }
    if ui.button("Copy path").clicked() {
        actions.push(Action::CopyPath(dir.to_path_buf()));
    }
    if ui.button(reveal_label()).clicked() {
        actions.push(Action::Reveal(dir.to_path_buf()));
    }
    if ui.button("Refresh").clicked() {
        actions.push(Action::Refresh(dir.to_path_buf()));
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        actions.push(Action::Delete(dir.to_path_buf()));
    }
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") { "Reveal in Finder" } else { "Reveal in file manager" }
}

fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}
