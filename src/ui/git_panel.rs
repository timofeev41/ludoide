//! Git panel: the everyday 80% — changes, stage/unstage/discard, commit,
//! pull/push/fetch and branches. Every git call runs on a background thread.

use super::{dim, section_header, small_icon_button};
use crate::app::{App, Confirm};
use crate::editor::Document;
use crate::fonts::icon;
use crate::git::{self, Branches, Change, Status};
use crate::languages::LangId;
use egui::RichText;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

enum Event {
    Status(PathBuf, Result<Status, String>),
    Branches(PathBuf, Result<Branches, String>),
    Done { label: String, result: Result<String, String> },
    Diff { title: String, text: Result<String, String> },
}

pub struct GitPanel {
    /// Repository shown (work tree root).
    pub repo: Option<PathBuf>,
    repos: Vec<PathBuf>,
    status: Option<Result<Status, String>>,
    branches: Option<Branches>,
    pub message: String,
    pub message_focused: bool,
    busy: Option<String>,
    refreshed_at: Option<Instant>,
    error: Option<String>,
    branch_filter: String,
    new_branch: String,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Default for GitPanel {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            repo: None,
            repos: Vec::new(),
            status: None,
            branches: None,
            message: String::new(),
            message_focused: false,
            busy: None,
            refreshed_at: None,
            error: None,
            branch_filter: String::new(),
            new_branch: String::new(),
            tx,
            rx,
        }
    }
}

/// What a button asks for; run on a thread by [`GitPanel::start`].
enum Op {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Commit { all: bool, amend: bool, push: bool },
    Pull,
    Push,
    Fetch,
    Checkout(String, bool),
    CreateBranch(String),
}

impl GitPanel {
    fn refresh(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repo.clone() else { return };
        self.refreshed_at = Some(Instant::now());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Status(repo.clone(), git::status(&repo)));
            ctx.request_repaint();
        });
    }

    fn load_branches(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repo.clone() else { return };
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Branches(repo.clone(), git::branches(&repo)));
            ctx.request_repaint();
        });
    }

    fn start(&mut self, op: Op, ctx: &egui::Context) {
        let (Some(repo), None) = (self.repo.clone(), &self.busy) else { return };
        let st = self.status.as_ref().and_then(|s| s.as_ref().ok()).cloned().unwrap_or_default();
        let label = match &op {
            Op::Stage(_) => "Stage",
            Op::Unstage(_) => "Unstage",
            Op::Commit { push: true, .. } => "Commit & push",
            Op::Commit { amend: true, .. } => "Amend",
            Op::Commit { .. } => "Commit",
            Op::Pull => "Pull",
            Op::Push => "Push",
            Op::Fetch => "Fetch",
            Op::Checkout(..) => "Checkout",
            Op::CreateBranch(_) => "Create branch",
        }
        .to_owned();
        let message = self.message.clone();
        self.busy = Some(label.clone());
        self.error = None;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = &repo;
            let with_paths = |cmd: &[&str], paths: &[String]| {
                let mut args: Vec<&str> = cmd.to_vec();
                args.push("--");
                args.extend(paths.iter().map(String::as_str));
                git::run(r, &args)
            };
            let result = match op {
                Op::Stage(paths) => with_paths(&["add", "-A"], &paths),
                Op::Unstage(paths) => with_paths(&["restore", "--staged"], &paths)
                    // A repository without commits has nothing to restore from.
                    .or_else(|_| with_paths(&["rm", "--cached", "-r", "-q"], &paths)),
                Op::Commit { all, amend, push } => (|| {
                    if all {
                        git::run(r, &["add", "-A"])?;
                    }
                    let mut args = vec!["commit", "-m", message.as_str()];
                    if amend {
                        args.push("--amend");
                        if message.trim().is_empty() {
                            args = vec!["commit", "--amend", "--no-edit"];
                        }
                    }
                    let out = git::run(r, &args)?;
                    if push { git::push(r, &git::status(r)?) } else { Ok(out) }
                })(),
                Op::Pull => git::run(r, &["pull"]),
                Op::Push => git::push(r, &st),
                Op::Fetch => git::run(r, &["fetch", "--all", "--prune"]),
                Op::Checkout(b, remote) => git::checkout(r, &b, remote),
                Op::CreateBranch(b) => git::run(r, &["checkout", "-b", &b]),
            };
            let _ = tx.send(Event::Done { label, result });
            let _ = tx.send(Event::Status(repo.clone(), git::status(&repo)));
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) -> Vec<(String, Result<String, String>)> {
        let mut docs = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Status(repo, st) if Some(&repo) == self.repo.as_ref() => self.status = Some(st),
                Event::Branches(repo, b) if Some(&repo) == self.repo.as_ref() => match b {
                    Ok(b) => self.branches = Some(b),
                    Err(e) => self.error = Some(e),
                },
                Event::Done { label, result } => {
                    self.busy = None;
                    match result {
                        Ok(_) => {
                            if label.starts_with("Commit") || label == "Amend" {
                                self.message.clear();
                            }
                            self.branches = None;
                        }
                        Err(e) => self.error = Some(format!("{label} failed: {e}")),
                    }
                }
                Event::Diff { title, text } => docs.push((title, text)),
                _ => {}
            }
        }
        docs
    }
}

/// Repositories in the workspace: each root's repository, or repositories one level below it.
fn find_repos(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        let found = match git::repo_root(root) {
            Some(r) => vec![r],
            None => std::fs::read_dir(root)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.join(".git").exists()).collect())
                .unwrap_or_default(),
        };
        for r in found {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out.sort();
    out
}

fn repo_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

impl App {
    pub fn git_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        for (title, text) in self.git_ui.poll() {
            match text {
                Ok(t) if t.trim().is_empty() => self.notify("No changes to show"),
                Ok(t) => self.open_document(Document::scratch(title, t, LangId::Diff)),
                Err(e) => self.error(e),
            }
        }
        // Keep the repository list and status fresh while the panel is visible.
        let st = &mut self.git_ui;
        let stale = st.refreshed_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3));
        if stale {
            let mut roots = self.workspace.roots.clone();
            if let Some(f) = self.tabs.current().and_then(|d| d.path.clone()) {
                roots.insert(0, f);
            }
            st.repos = find_repos(&roots);
            let current = self.tabs.current().and_then(|d| d.path.as_deref()).and_then(git::repo_root);
            if st.repo.as_ref().is_none_or(|r| !st.repos.contains(r)) {
                st.repo = current.or_else(|| st.repos.first().cloned());
                st.status = None;
                st.branches = None;
            }
            st.refresh(&ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(3));

        let mut confirm = None;
        let mut open: Option<PathBuf> = None;
        let mut diff: Option<(Change, bool)> = None;
        ui.horizontal(|ui| {
            section_header(ui, "Source control", &p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if small_icon_button(ui, icon::ARROWS_CLOCKWISE, "Refresh", &p).clicked() {
                    st.refresh(&ctx);
                }
                if small_icon_button(ui, icon::CLOUD_ARROW_DOWN, "Fetch (all remotes)", &p).clicked() {
                    st.start(Op::Fetch, &ctx);
                }
            });
        });
        if st.repos.is_empty() {
            ui.add_space(8.0);
            ui.label(dim("No git repository in the workspace.", &p));
            return;
        }
        if st.repos.len() > 1 {
            let selected = st.repo.as_deref().map(repo_name).unwrap_or_default();
            let mut pick = None;
            egui::ComboBox::from_id_salt("git_repo")
                .selected_text(format!("{} {selected}", icon::GIT_DIFF))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for r in &st.repos {
                        if ui
                            .selectable_label(st.repo.as_ref() == Some(r), repo_name(r))
                            .on_hover_text(r.display().to_string())
                            .clicked()
                        {
                            pick = Some(r.clone());
                        }
                    }
                });
            if let Some(r) = pick {
                st.repo = Some(r);
                st.status = None;
                st.branches = None;
                st.refresh(&ctx);
            }
        }
        let status = match &st.status {
            None => {
                ui.spinner();
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(p.error, e);
                return;
            }
            Some(Ok(s)) => s.clone(),
        };
        let busy = st.busy.is_some();

        // Branch + sync row.
        ui.horizontal(|ui| {
            // Stays open while typing a branch name or filter; closes on a click outside.
            let config =
                egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            let branch_button =
                RichText::new(format!("{} {}", icon::GIT_BRANCH, super::short(&status.branch, 20))).strong();
            let (resp, _) = egui::containers::menu::MenuButton::new(branch_button).config(config).ui(ui, |ui| {
                ui.set_min_width(260.0);
                ui.set_max_width(320.0);
                if st.branches.is_none() {
                    st.load_branches(&ctx);
                    st.branches = Some(Branches::default());
                }
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut st.new_branch)
                            .hint_text("new-branch-name")
                            .desired_width(170.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let name = st.new_branch.trim().to_owned();
                    if (ui
                        .add_enabled(!name.is_empty(), egui::Button::new("Create"))
                        .on_hover_text("Create from the current branch and switch to it")
                        .clicked()
                        || enter)
                        && !name.is_empty()
                    {
                        st.start(Op::CreateBranch(name), &ctx);
                        st.new_branch.clear();
                        ui.close();
                    }
                });
                ui.separator();
                super::search_field(ui, &mut st.branch_filter, "Filter branches", &p);
                let f = st.branch_filter.to_lowercase();
                let b = st.branches.clone().unwrap_or_default();
                let mut pick = None;
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for name in b.local.iter().filter(|n| n.to_lowercase().contains(&f)) {
                        let current = *name == status.branch;
                        let text = if current { format!("{} {name}", icon::CHECK) } else { format!("    {name}") };
                        if ui.selectable_label(current, text).clicked() && !current {
                            pick = Some((name.clone(), false));
                        }
                    }
                    let remote: Vec<&String> = b.remote.iter().filter(|n| n.to_lowercase().contains(&f)).collect();
                    if !remote.is_empty() {
                        ui.label(dim("Remote", &p).small());
                        for name in remote {
                            if ui
                                .selectable_label(false, format!("{} {name}", icon::CLOUD))
                                .on_hover_text("Check out as a local tracking branch")
                                .clicked()
                            {
                                pick = Some((name.clone(), true));
                            }
                        }
                    }
                    if b.local.is_empty() && b.remote.is_empty() {
                        ui.spinner();
                    }
                });
                if let Some((name, remote)) = pick {
                    st.start(Op::Checkout(name, remote), &ctx);
                    ui.close();
                }
            });
            resp.on_hover_text(match &status.upstream {
                Some(u) => format!("{}\nSwitch or create a branch · tracking {u}", status.branch),
                None => {
                    format!("{}\nSwitch or create a branch · not published yet (Push sets the upstream)", status.branch)
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let push_label = if status.ahead > 0 {
                    format!("{} Push {}", icon::ARROW_UP, status.ahead)
                } else {
                    format!("{} Push", icon::ARROW_UP)
                };
                if ui
                    .add_enabled(!busy, egui::Button::new(push_label))
                    .on_hover_text(if status.upstream.is_some() {
                        "git push"
                    } else {
                        "Publish the branch (git push -u)"
                    })
                    .clicked()
                {
                    st.start(Op::Push, &ctx);
                }
                let pull_label = if status.behind > 0 {
                    format!("{} Pull {}", icon::ARROW_DOWN, status.behind)
                } else {
                    format!("{} Pull", icon::ARROW_DOWN)
                };
                if ui.add_enabled(!busy, egui::Button::new(pull_label)).on_hover_text("git pull").clicked() {
                    st.start(Op::Pull, &ctx);
                }
            });
        });

        // Commit box.
        let staged: Vec<&Change> = status.changes.iter().filter(|c| c.staged()).collect();
        let unstaged: Vec<&Change> = status.changes.iter().filter(|c| c.unstaged()).collect();
        let msg = ui.add(
            egui::TextEdit::multiline(&mut st.message)
                .hint_text(format!("Message (⌘↵ to commit on '{}')", status.branch))
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        st.message_focused = msg.has_focus();
        let all = staged.is_empty();
        let can_commit = !busy && !st.message.trim().is_empty() && !status.changes.is_empty();
        // Right to left: the ▾ menu first, then the button fills exactly what is left.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 4.0);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(icon::CARET_DOWN, |ui| {
                if ui.add_enabled(can_commit, egui::Button::new("Commit & push")).clicked() {
                    st.start(Op::Commit { all, amend: false, push: true }, &ctx);
                    ui.close();
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Amend last commit"))
                    .on_hover_text("Adds staged changes to the last commit; an empty message keeps its message")
                    .clicked()
                {
                    st.start(Op::Commit { all: false, amend: true, push: false }, &ctx);
                    ui.close();
                }
            });
            let label = if all && !unstaged.is_empty() { "Commit all" } else { "Commit" };
            let tip =
                if all { "Nothing staged: stages every change, then commits" } else { "Commit the staged changes" };
            let button = egui::Button::new(RichText::new(format!("{} {label}", icon::CHECK)).strong())
                .min_size(egui::vec2(ui.available_width(), 0.0));
            if ui.add_enabled(can_commit, button).on_hover_text(tip).clicked() {
                st.start(Op::Commit { all, amend: false, push: false }, &ctx);
            }
        });
        if let Some(b) = &st.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(dim(format!("{b}…"), &p).small());
            });
        }
        if let Some(e) = st.error.clone() {
            ui.horizontal_top(|ui| {
                ui.add(egui::Label::new(RichText::new(&e).small().color(p.error)).wrap());
                if small_icon_button(ui, icon::X, "Dismiss", &p).clicked() {
                    st.error = None;
                }
            });
        }
        ui.add_space(4.0);

        let repo = st.repo.clone().unwrap_or_default();
        let mut op = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if status.changes.is_empty() {
                ui.label(dim("No changes.", &p));
            }
            for (staged_list, title, list) in [(true, "Staged changes", &staged), (false, "Changes", &unstaged)] {
                if list.is_empty() {
                    continue;
                }
                let header = egui::CollapsingHeader::new(RichText::new(format!("{title}  {}", list.len())).strong())
                    .id_salt(("git_section", staged_list))
                    .default_open(true)
                    .show(ui, |ui| {
                        for c in list.iter() {
                            ui.horizontal(|ui| {
                                let letter = c.letter(staged_list);
                                let color = match letter {
                                    'A' | 'U' => p.success,
                                    'D' => p.error,
                                    _ => p.warning,
                                };
                                let (dir, name) = match c.path.rsplit_once('/') {
                                    Some((d, n)) => (d, n),
                                    None => ("", c.path.as_str()),
                                };
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(letter.to_string()).monospace().color(color));
                                    if staged_list {
                                        if small_icon_button(ui, icon::MINUS, "Unstage", &p).clicked() {
                                            op = Some(Op::Unstage(vec![c.path.clone()]));
                                        }
                                    } else {
                                        if small_icon_button(ui, icon::PLUS, "Stage", &p).clicked() {
                                            op = Some(Op::Stage(vec![c.path.clone()]));
                                        }
                                        if small_icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, "Discard changes", &p)
                                            .clicked()
                                        {
                                            confirm = Some(vec![(*c).clone()]);
                                        }
                                    }
                                    if letter != 'D'
                                        && small_icon_button(ui, icon::FILE_TEXT, "Open file", &p).clicked()
                                    {
                                        open = Some(repo.join(&c.path));
                                    }
                                    // Name and folder take what the buttons leave, cut with "…".
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                        let r = ui
                                            .add(
                                                egui::Label::new(RichText::new(format!(
                                                    "{} {name}",
                                                    super::files::file_icon(Path::new(name))
                                                )))
                                                .sense(egui::Sense::click())
                                                .truncate(),
                                            )
                                            .on_hover_text(format!("{}\nClick: diff · Right-click: more", c.path));
                                        if r.clicked() {
                                            diff = Some(((*c).clone(), staged_list));
                                        }
                                        r.context_menu(|ui| {
                                            if ui.button("Open diff").clicked() {
                                                diff = Some(((*c).clone(), staged_list));
                                            }
                                            if letter != 'D' && ui.button("Open file").clicked() {
                                                open = Some(repo.join(&c.path));
                                            }
                                            if ui.button("Copy path").clicked() {
                                                ui.ctx().copy_text(repo.join(&c.path).display().to_string());
                                            }
                                        });
                                        if !dir.is_empty() {
                                            ui.add(egui::Label::new(dim(dir, &p).small()).truncate());
                                        }
                                    });
                                });
                            });
                        }
                    });
                let paths: Vec<String> = list.iter().map(|c| c.path.clone()).collect();
                header.header_response.context_menu(|ui| {
                    if staged_list {
                        if ui.button("Unstage all").clicked() {
                            op = Some(Op::Unstage(paths.clone()));
                        }
                    } else {
                        if ui.button("Stage all").clicked() {
                            op = Some(Op::Stage(paths.clone()));
                        }
                        if ui.button(RichText::new("Discard all changes").color(p.error)).clicked() {
                            confirm = Some(list.iter().map(|c| (*c).clone()).collect());
                        }
                    }
                });
            }
        });
        if let Some(op) = op {
            st.start(op, &ctx);
        }
        if let Some(path) = open {
            self.open_file(&path);
        }
        if let Some((c, staged)) = diff {
            let tx = self.git_ui.tx.clone();
            let title = format!(
                "{} ({})",
                c.path.rsplit('/').next().unwrap_or(&c.path),
                if staged { "staged" } else { "changes" }
            );
            std::thread::spawn(move || {
                let _ = tx.send(Event::Diff { title, text: git::diff(&repo, &c, staged) });
                ctx.request_repaint();
            });
        }
        if let Some(changes) = confirm {
            if let Some(repo) = self.git_ui.repo.clone() {
                self.confirm = Some(Confirm::GitDiscard(repo, changes));
            }
        }
    }

    /// ⌘↵ in the commit message box.
    pub fn git_commit_shortcut(&mut self, ctx: &egui::Context) {
        let st = &mut self.git_ui;
        let Some(Ok(status)) = &st.status else { return };
        if st.message.trim().is_empty() || status.changes.is_empty() {
            return;
        }
        let all = !status.changes.iter().any(|c| c.staged());
        st.start(Op::Commit { all, amend: false, push: false }, ctx);
    }

    pub fn git_discard(&mut self, repo: &Path, changes: &[Change]) {
        match git::discard(repo, changes) {
            Ok(_) => {
                for doc in &mut self.tabs.docs {
                    doc.reload_if_changed();
                }
                self.workspace.invalidate_all();
            }
            Err(e) => self.error(format!("Discard failed: {e}")),
        }
        self.git_ui.refreshed_at = None;
    }
}
