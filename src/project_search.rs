//! Find in project (⌘F): file names and contents across the workspace roots
//! and Scratches. Runs on a background thread; a newer search cancels older ones.

use crate::file_filter::PathFilter;
use crate::ui::find::find_all;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "target",
    "dbt_packages",
    "dbt_modules",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".idea",
    ".tox",
];
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LINE_HITS: usize = 5_000;
const PREVIEW_CHARS: usize = 160;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    /// Explorer filter syntax (`.sql|.yml`, `models/`), empty = all files.
    pub include: String,
    pub show_hidden: bool,
}

#[derive(Clone, Debug)]
pub struct LineHit {
    /// 1-based.
    pub line: usize,
    /// The line, trimmed and shortened for display.
    pub preview: String,
    /// Match position inside `preview` (chars).
    pub preview_range: (usize, usize),
    /// Match position in the whole file (chars), for selecting it.
    pub range: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct FileHit {
    pub path: PathBuf,
    pub root: PathBuf,
    pub name_match: bool,
    pub lines: Vec<LineHit>,
}

#[derive(Clone, Debug, Default)]
pub struct Results {
    pub files: Vec<FileHit>,
    pub line_hits: usize,
    pub truncated: bool,
}

/// Search `roots`. Stops early (returning `None`) once `generation` moves past `my_generation`.
pub fn search(roots: &[PathBuf], q: &Query, generation: &AtomicU64, my_generation: u64) -> Option<Results> {
    let include = PathFilter::parse(&q.include);
    let needle = if q.case_sensitive { q.text.clone() } else { q.text.to_lowercase() };
    let mut res = Results::default();
    for root in roots {
        let mut stack = vec![(root.clone(), Vec::<String>::new())];
        while let Some((dir, ancestors)) = stack.pop() {
            if generation.load(Ordering::Relaxed) != my_generation {
                return None;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            // Visit subdirectories in name order (the stack pops in reverse).
            let mut subdirs = Vec::new();
            for e in entries {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') && !q.show_hidden && name != ".env" {
                    continue;
                }
                let path = e.path();
                let Ok(meta) = e.metadata() else { continue };
                if meta.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        let mut a = ancestors.clone();
                        a.push(name.to_lowercase());
                        subdirs.push((path, a));
                    }
                    continue;
                }
                if !include.is_empty()
                    && !include.file_matches(&name, &ancestors)
                    && !ancestors.iter().any(|a| include.dir_matches(a))
                {
                    continue;
                }
                let name_cmp = if q.case_sensitive { name.clone() } else { name.to_lowercase() };
                let name_match = name_cmp.contains(&needle);
                let lines = if res.line_hits < MAX_LINE_HITS && meta.len() <= MAX_FILE_BYTES {
                    search_file(&path, q, MAX_LINE_HITS - res.line_hits)
                } else {
                    Vec::new()
                };
                if res.line_hits + lines.len() >= MAX_LINE_HITS {
                    res.truncated = true;
                }
                if name_match || !lines.is_empty() {
                    res.line_hits += lines.len();
                    res.files.push(FileHit { path, root: root.clone(), name_match, lines });
                }
            }
            stack.extend(subdirs.into_iter().rev());
        }
    }
    Some(res)
}

fn search_file(path: &Path, q: &Query, limit: usize) -> Vec<LineHit> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    // Binary files: skip.
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Vec::new();
    }
    let Ok(text) = String::from_utf8(bytes) else { return Vec::new() };
    let mut out = Vec::new();
    let mut offset = 0; // chars before the current line
    for (i, line) in text.split('\n').enumerate() {
        let len = line.chars().count();
        for (a, b) in find_all(line, &q.text, q.case_sensitive) {
            if out.len() >= limit {
                return out;
            }
            let (preview, preview_range) = preview(line, (a, b));
            out.push(LineHit { line: i + 1, preview, preview_range, range: (offset + a, offset + b) });
        }
        offset += len + 1;
    }
    out
}

/// The line without leading whitespace, cut around the match if it is long.
fn preview(line: &str, (a, b): (usize, usize)) -> (String, (usize, usize)) {
    let chars: Vec<char> = line.trim_end().chars().collect();
    let lead = chars.iter().take_while(|c| c.is_whitespace()).count().min(a);
    let mut start = lead;
    // Start close to the match so it stays visible in a narrow panel.
    if a - start > 24 {
        start = a - 16;
    }
    let end = (start + PREVIEW_CHARS).min(chars.len()).max(b.min(chars.len()));
    let mut s: String = chars[start..end].iter().collect();
    let mut shift = start;
    if start > lead {
        s.insert(0, '…');
        shift -= 1;
    }
    if end < chars.len() {
        s.push('…');
    }
    (s, (a - shift, b.min(end) - shift))
}

/// Background runner: the UI submits queries and polls for the latest result.
pub struct Searcher {
    generation: Arc<AtomicU64>,
    tx: std::sync::mpsc::Sender<(u64, Option<Results>)>,
    rx: std::sync::mpsc::Receiver<(u64, Option<Results>)>,
}

impl Default for Searcher {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { generation: Arc::new(AtomicU64::new(0)), tx, rx }
    }
}

impl Searcher {
    pub fn submit(&self, roots: Vec<PathBuf>, q: Query, ctx: egui::Context) -> u64 {
        let my_generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = self.generation.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = search(&roots, &q, &generation, my_generation);
            if r.is_some() {
                let _ = tx.send((my_generation, r));
                ctx.request_repaint();
            }
        });
        my_generation
    }

    /// The newest finished result, if it belongs to `generation`.
    pub fn poll(&self, generation: u64) -> Option<Results> {
        let mut latest = None;
        while let Ok((g, r)) = self.rx.try_recv() {
            if g == generation {
                latest = r;
            }
        }
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_and_contents() {
        let dir = std::env::temp_dir().join(format!("ludo-find-{}", crate::db::new_id()));
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("models/orders.sql"), "select 1\n  from {{ ref('orders') }}\n").unwrap();
        std::fs::write(dir.join("models/users.yml"), "name: users\ncolumns: [ORDERS]\n").unwrap();
        std::fs::write(dir.join("target/orders.sql"), "orders").unwrap();
        let q = Query { text: "orders".into(), ..Default::default() };
        let r = search(&[dir.clone()], &q, &AtomicU64::new(0), 0).unwrap();
        assert_eq!(r.files.len(), 2, "{r:?}");
        let sql = r.files.iter().find(|f| f.path.ends_with("orders.sql")).unwrap();
        assert!(sql.name_match);
        assert_eq!(sql.lines[0].line, 2);
        assert_eq!(sql.lines[0].range, (24, 30));
        assert_eq!(&sql.lines[0].preview, "from {{ ref('orders') }}");
        let p = &sql.lines[0];
        assert_eq!(p.preview.chars().skip(p.preview_range.0).take(6).collect::<String>(), "orders");
        // Include filter + case sensitivity.
        let q = Query { text: "ORDERS".into(), case_sensitive: true, include: ".yml".into(), ..Default::default() };
        let r = search(&[dir.clone()], &q, &AtomicU64::new(0), 0).unwrap();
        assert_eq!(r.files.len(), 1);
        assert!(r.files[0].path.ends_with("users.yml"));
        // A newer generation cancels.
        assert!(search(&[dir.clone()], &q, &AtomicU64::new(5), 0).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn previews_long_lines() {
        let line = format!("{}needle{}", "x".repeat(300), "y".repeat(300));
        let (p, (a, b)) = preview(&line, (300, 306));
        assert!(p.starts_with('…') && p.ends_with('…'));
        assert_eq!(p.chars().skip(a).take(b - a).collect::<String>(), "needle");
    }
}
