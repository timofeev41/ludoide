//! Explorer filter, with the same syntax as the Databases search.
//!
//! - Words separated by `|` or spaces are alternatives: `.sql|.yml`.
//! - A plain word matches file *and* folder names; a matching folder shows
//!   everything inside it.
//! - `dir/file` narrows: `models/orders` = files containing "orders" inside a
//!   folder containing "models"; `models/` = folders only; `/orders` = files only.
//! - Matching is case-insensitive substring. Folders without matches are hidden.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
struct Term {
    dir: Option<String>,
    file: Option<String>,
    /// Written without a slash: matches a file or a folder.
    either: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathFilter {
    terms: Vec<Term>,
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim().to_lowercase();
    (!s.is_empty()).then_some(s)
}

impl PathFilter {
    pub fn parse(q: &str) -> Self {
        let terms = q
            .split(|c: char| c == '|' || c.is_whitespace())
            .map(str::trim)
            .filter(|t| !t.is_empty() && *t != "/")
            .map(|t| match t.rsplit_once('/') {
                Some((d, f)) => Term { dir: opt(d.trim_matches('/')), file: opt(f), either: false },
                None => Term { dir: opt(t), file: opt(t), either: true },
            })
            .collect();
        Self { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The folder's own name matches, so all of its contents are shown.
    pub fn dir_matches(&self, name: &str) -> bool {
        let l = name.to_lowercase();
        self.terms
            .iter()
            .any(|t| (t.either || t.file.is_none()) && t.dir.as_ref().is_some_and(|d| l.contains(d.as_str())))
    }

    /// `ancestors`: lowercased names of the folders between the root and the file.
    pub fn file_matches(&self, name: &str, ancestors: &[String]) -> bool {
        let l = name.to_lowercase();
        self.terms.iter().any(|t| {
            let Some(f) = &t.file else { return false };
            if !l.contains(f.as_str()) {
                return false;
            }
            t.either || t.dir.as_ref().is_none_or(|d| ancestors.iter().any(|a| a.contains(d.as_str())))
        })
    }
}

/// What the Explorer shows while filtering.
#[derive(Default, Debug)]
pub struct Visible {
    /// Files and folders to show.
    pub shown: HashSet<PathBuf>,
    /// Folders whose whole contents are shown.
    pub full: HashSet<PathBuf>,
}

/// Folders not searched into (still shown if their own name matches).
const HEAVY: &[&str] = &["node_modules", ".venv", "venv", "target", "dbt_packages", "dbt_modules", ".tox"];
const MAX_ENTRIES: usize = 50_000;

impl Visible {
    /// Walk `root` with `list(dir) -> [(path, name, is_dir)]`.
    pub fn compute(
        filter: &PathFilter,
        roots: &[PathBuf],
        mut list: impl FnMut(&Path) -> Vec<(PathBuf, String, bool)>,
    ) -> Self {
        let mut v = Visible::default();
        let mut budget = MAX_ENTRIES;
        for root in roots {
            let mut ancestors = Vec::new();
            if v.visit(filter, root, &mut ancestors, &mut list, &mut budget) {
                v.shown.insert(root.clone());
            }
        }
        v
    }

    fn visit(
        &mut self,
        filter: &PathFilter,
        dir: &Path,
        ancestors: &mut Vec<String>,
        list: &mut impl FnMut(&Path) -> Vec<(PathBuf, String, bool)>,
        budget: &mut usize,
    ) -> bool {
        let mut any = false;
        for (path, name, is_dir) in list(dir) {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let show = if is_dir {
                if filter.dir_matches(&name) {
                    self.full.insert(path.clone());
                    true
                } else if HEAVY.contains(&name.as_str()) {
                    false
                } else {
                    ancestors.push(name.to_lowercase());
                    let r = self.visit(filter, &path, ancestors, list, budget);
                    ancestors.pop();
                    r
                }
            } else {
                filter.file_matches(&name, ancestors)
            };
            if show {
                self.shown.insert(path);
                any = true;
            }
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Vec<(PathBuf, Vec<(PathBuf, String, bool)>)> {
        let e = |p: &str, d: bool| (PathBuf::from(p), p.rsplit('/').next().unwrap().to_owned(), d);
        vec![
            (PathBuf::from("/r"), vec![e("/r/a", true), e("/r/b", true), e("/r/c", true), e("/r/models", true)]),
            (PathBuf::from("/r/a"), vec![e("/r/a/test.sql", false)]),
            (PathBuf::from("/r/b"), vec![e("/r/b/s.py", false)]),
            (PathBuf::from("/r/c"), vec![e("/r/c/a.yaml", false), e("/r/c/test.py", false)]),
            (PathBuf::from("/r/models"), vec![e("/r/models/orders.sql", false), e("/r/models/users.sql", false)]),
        ]
    }

    fn run(q: &str) -> Visible {
        let t = tree();
        Visible::compute(&PathFilter::parse(q), &[PathBuf::from("/r")], |d| {
            t.iter().find(|(p, _)| p == d).map(|(_, c)| c.clone()).unwrap_or_default()
        })
    }

    #[test]
    fn hides_folders_without_matches() {
        let v = run(".sql");
        assert!(v.shown.contains(Path::new("/r/a/test.sql")) && v.shown.contains(Path::new("/r/a")));
        assert!(!v.shown.contains(Path::new("/r/b")) && !v.shown.contains(Path::new("/r/c")));
        assert!(v.shown.contains(Path::new("/r")));
    }

    #[test]
    fn alternatives_and_folders() {
        let v = run("a.yaml|s.py");
        assert!(v.shown.contains(Path::new("/r/c/a.yaml")) && v.shown.contains(Path::new("/r/b/s.py")));
        assert!(!v.shown.contains(Path::new("/r/c/test.py")));
        // A matching folder shows everything inside.
        let v = run("models");
        assert!(v.full.contains(Path::new("/r/models")) && v.shown.contains(Path::new("/r/models")));
    }

    #[test]
    fn slashes() {
        let v = run("models/orders");
        assert!(v.shown.contains(Path::new("/r/models/orders.sql")));
        assert!(!v.shown.contains(Path::new("/r/models/users.sql")) && v.full.is_empty());
        let v = run("/test");
        assert!(v.shown.contains(Path::new("/r/a/test.sql")) && v.shown.contains(Path::new("/r/c/test.py")));
        assert!(run("nothing").shown.is_empty());
        assert!(PathFilter::parse(" | ").is_empty());
    }
}
