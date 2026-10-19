pub mod highlight;
pub mod ops;
pub mod view;

use crate::languages::{self, LangId};
use crate::lint::Diagnostic;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

pub const MAX_TABS: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MdMode {
    Edit,
    Split,
    Preview,
}

pub struct Document {
    pub uid: u64,
    pub path: Option<PathBuf>,
    /// Title for documents without a path (e.g. DDL views).
    pub title: String,
    pub text: String,
    pub lang: LangId,
    pub version: u64,
    pub saved_version: u64,
    pub disk_mtime: Option<SystemTime>,
    pub last_edit: Instant,
    pub last_access: Instant,
    pub diagnostics: Vec<Diagnostic>,
    pub linted_version: Option<u64>,
    pub md_mode: MdMode,
    /// Current selection in chars (start <= end), updated by the view.
    pub sel: (usize, usize),
    pub cursor_line_col: (usize, usize),
    /// `(text hash, font size, theme id)` -> highlighted job.
    pub(crate) highlight_cache: Option<(u64, u32, &'static str, egui::text::LayoutJob)>,
    /// Scroll the editor to show this line on the next frame (1-based).
    pub goto_line: Option<usize>,
    /// Frames left during which the editor (re)claims keyboard focus; a click that
    /// opened the document would otherwise take focus away again.
    pub wants_focus: u8,
    /// Input events deferred to the next frame to keep typing order intact.
    pub(crate) deferred_events: Vec<egui::Event>,
    /// Selection to apply on the next frame (char range), e.g. from Find.
    pub pending_select: Option<(usize, usize)>,
    /// dbt project this file belongs to (enables Compile).
    pub dbt: Option<crate::dbt::DbtProject>,
    /// Model name from a ⌘-click on `ref('…')`, handled by the app.
    pub goto_ref: Option<String>,
}

fn next_uid() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

impl Document {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "file is not valid UTF-8 text"))?;
        let lang = languages::detect(path);
        let mut doc = Self::scratch(String::new(), text, lang);
        doc.disk_mtime = mtime(path);
        doc.path = Some(path.to_path_buf());
        doc.dbt = crate::dbt::DbtProject::for_file(path);
        doc.md_mode = if lang == LangId::Markdown { MdMode::Split } else { MdMode::Edit };
        Ok(doc)
    }

    pub fn scratch(title: String, text: String, lang: LangId) -> Self {
        Self {
            uid: next_uid(),
            path: None,
            title,
            text: text.replace("\r\n", "\n"),
            lang,
            version: 0,
            saved_version: 0,
            disk_mtime: None,
            last_edit: Instant::now(),
            last_access: Instant::now(),
            diagnostics: Vec::new(),
            linted_version: None,
            md_mode: MdMode::Edit,
            sel: (0, 0),
            cursor_line_col: (1, 1),
            highlight_cache: None,
            goto_line: None,
            wants_focus: 3,
            deferred_events: Vec::new(),
            pending_select: None,
            dbt: None,
            goto_ref: None,
        }
    }

    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.title.clone())
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved_version && self.path.is_some()
    }

    pub fn mark_edited(&mut self) {
        self.version += 1;
        self.last_edit = Instant::now();
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            std::fs::write(path, &self.text)?;
            self.saved_version = self.version;
            self.disk_mtime = mtime(path);
        }
        Ok(())
    }

    /// Reload from disk when the file changed externally and has no local edits.
    pub fn reload_if_changed(&mut self) -> bool {
        let Some(path) = self.path.clone() else { return false };
        let m = mtime(&path);
        if m.is_none() || m == self.disk_mtime || self.is_dirty() {
            return false;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.text = text.replace("\r\n", "\n");
                self.disk_mtime = m;
                self.mark_edited();
                self.saved_version = self.version;
                true
            }
            Err(_) => false,
        }
    }

    pub fn selected_text(&self) -> String {
        self.text.chars().skip(self.sel.0).take(self.sel.1 - self.sel.0).collect()
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

#[derive(Default)]
pub struct Tabs {
    pub docs: Vec<Document>,
    pub active: usize,
}

pub enum OpenOutcome {
    Opened,
    /// Opened after closing the least recently used, unmodified tab.
    Evicted(String),
}

impl Tabs {
    pub fn current(&self) -> Option<&Document> {
        self.docs.get(self.active)
    }

    pub fn current_mut(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    pub fn activate(&mut self, i: usize) {
        if i < self.docs.len() {
            self.active = i;
            self.docs[i].last_access = Instant::now();
            self.docs[i].wants_focus = 3;
        }
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        self.docs.iter().position(|d| d.path.as_deref() == Some(path))
    }

    /// Make room for one more tab, honouring [`MAX_TABS`].
    fn make_room(&mut self) -> Result<Option<String>, String> {
        if self.docs.len() < MAX_TABS {
            return Ok(None);
        }
        let victim = self
            .docs
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.is_dirty())
            .min_by_key(|(_, d)| d.last_access)
            .map(|(i, _)| i);
        match victim {
            Some(i) => {
                let name = self.docs[i].name();
                self.close(i);
                Ok(Some(name))
            }
            None => Err(format!("All {MAX_TABS} tabs have unsaved changes — save or close one first")),
        }
    }

    pub fn open_path(&mut self, path: &Path) -> Result<OpenOutcome, String> {
        if let Some(i) = self.find(path) {
            self.activate(i);
            return Ok(OpenOutcome::Opened);
        }
        let doc = Document::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.push(doc)
    }

    pub fn push(&mut self, doc: Document) -> Result<OpenOutcome, String> {
        let evicted = self.make_room()?;
        self.docs.push(doc);
        self.activate(self.docs.len() - 1);
        Ok(evicted.map_or(OpenOutcome::Opened, OpenOutcome::Evicted))
    }

    pub fn close(&mut self, i: usize) {
        if i >= self.docs.len() {
            return;
        }
        self.docs.remove(i);
        if self.active > i || self.active >= self.docs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    pub fn cycle(&mut self, forward: bool) {
        let n = self.docs.len();
        if n > 1 {
            let i = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
            self.activate(i);
        }
    }

    /// Update paths after a file or directory was renamed on disk.
    pub fn rename_path(&mut self, from: &Path, to: &Path) {
        for d in &mut self.docs {
            if let Some(p) = &d.path {
                if let Ok(rest) = p.strip_prefix(from) {
                    let np = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                    d.lang = languages::detect(&np);
                    d.dbt = crate::dbt::DbtProject::for_file(&np);
                    d.path = Some(np);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_lru_clean_tab() {
        let mut tabs = Tabs::default();
        for i in 0..MAX_TABS {
            tabs.push(Document::scratch(format!("t{i}"), String::new(), LangId::Sql)).ok().unwrap();
        }
        tabs.activate(0);
        let out = tabs.push(Document::scratch("new".into(), String::new(), LangId::Sql)).ok().unwrap();
        assert!(matches!(out, OpenOutcome::Evicted(ref n) if n == "t1"));
        assert_eq!(tabs.docs.len(), MAX_TABS);
        tabs.cycle(true);
        assert_eq!(tabs.active, 0);
    }
}
