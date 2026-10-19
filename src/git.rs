//! Minimal git info for the status bar (no git binary needed).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Current branch (or short detached SHA) of the repository containing `path`.
pub fn branch(path: &Path) -> Option<String> {
    let mut dir = Some(path);
    while let Some(d) = dir {
        let git = d.join(".git");
        let git_dir = if git.is_dir() {
            git
        } else if git.is_file() {
            // Worktrees/submodules: `.git` is a file pointing at the real dir.
            let content = std::fs::read_to_string(&git).ok()?;
            let rel = content.trim().strip_prefix("gitdir:")?.trim();
            d.join(rel)
        } else {
            dir = d.parent();
            continue;
        };
        let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
        let head = head.trim();
        return Some(match head.strip_prefix("ref: refs/heads/") {
            Some(b) => b.to_owned(),
            None => head.chars().take(8).collect(),
        });
    }
    None
}

#[derive(Default)]
pub struct GitCache {
    key: Option<PathBuf>,
    value: Option<String>,
    at: Option<Instant>,
}

impl GitCache {
    pub fn get(&mut self, path: Option<&Path>) -> Option<String> {
        let path = path?;
        let fresh = self.at.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        if self.key.as_deref() != Some(path) || !fresh {
            self.value = branch(path);
            self.key = Some(path.to_path_buf());
            self.at = Some(Instant::now());
        }
        self.value.clone()
    }
}
