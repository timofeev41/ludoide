//! Git: the branch for the status bar (read straight from `.git`, no binary
//! needed) and the basic operations behind the Git panel (via the `git` CLI).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The `.git` directory of the repository containing `path`, and the work tree root.
fn find_git_dir(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut dir = Some(path);
    while let Some(d) = dir {
        let git = d.join(".git");
        if git.is_dir() {
            return Some((git, d.to_path_buf()));
        }
        if git.is_file() {
            // Worktrees/submodules: `.git` is a file pointing at the real dir.
            let content = std::fs::read_to_string(&git).ok()?;
            let rel = content.trim().strip_prefix("gitdir:")?.trim();
            return Some((d.join(rel), d.to_path_buf()));
        }
        dir = d.parent();
    }
    None
}

/// Work tree root of the repository containing `path`.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    find_git_dir(path).map(|(_, root)| root)
}

/// Current branch (or short detached SHA) of the repository containing `path`.
pub fn branch(path: &Path) -> Option<String> {
    let (git_dir, _) = find_git_dir(path)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(b) => b.to_owned(),
        None => head.chars().take(8).collect(),
    })
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

// ----- git CLI -----

/// Run git in `repo`. Never prompts: a missing credential fails instead of hanging.
pub fn run(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh -o BatchMode=yes".into()))
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("Cannot run git: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() { stdout.trim().to_owned() } else { err })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// Repository-relative path.
    pub path: String,
    /// Status letters as in `git status --short`: index (staged) and work tree.
    pub index: char,
    pub worktree: char,
}

impl Change {
    pub fn untracked(&self) -> bool {
        self.index == '?'
    }
    pub fn staged(&self) -> bool {
        !matches!(self.index, ' ' | '?' | '!')
    }
    pub fn unstaged(&self) -> bool {
        self.worktree != ' ' || self.untracked()
    }
    /// One letter for the list: U(ntracked), A, M, D, R…
    pub fn letter(&self, staged: bool) -> char {
        match (staged, self.untracked()) {
            (_, true) => 'U',
            (true, _) => self.index,
            (false, _) => self.worktree,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
}

/// Parse `git status --porcelain=v1 -b -z`.
pub fn parse_status(out: &str) -> Status {
    let mut st = Status::default();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        if let Some(head) = f.strip_prefix("## ") {
            parse_branch_line(head, &mut st);
            continue;
        }
        if f.len() < 4 {
            continue;
        }
        let mut chars = f.chars();
        let index = chars.next().unwrap_or(' ');
        let worktree = chars.next().unwrap_or(' ');
        let path = f[3..].to_owned();
        if matches!(index, 'R' | 'C') {
            fields.next(); // original path
        }
        st.changes.push(Change { path, index, worktree });
    }
    st
}

fn parse_branch_line(head: &str, st: &mut Status) {
    let (names, counts) = match head.split_once(" [") {
        Some((n, c)) => (n, c.trim_end_matches(']')),
        None => (head, ""),
    };
    let names =
        names.strip_prefix("No commits yet on ").or_else(|| names.strip_prefix("Initial commit on ")).unwrap_or(names);
    match names.split_once("...") {
        Some((b, u)) => {
            st.branch = b.to_owned();
            st.upstream = Some(u.to_owned());
        }
        None => st.branch = names.to_owned(),
    }
    if st.branch == "HEAD (no branch)" {
        st.branch = "detached HEAD".into();
    }
    for part in counts.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            st.ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            st.behind = n.parse().unwrap_or(0);
        } else if part == "gone" {
            st.upstream = None;
        }
    }
}

pub fn status(repo: &Path) -> Result<Status, String> {
    run(repo, &["status", "--porcelain=v1", "-b", "-z", "--untracked-files=all"]).map(|o| parse_status(&o))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Branches {
    pub local: Vec<String>,
    /// Remote branches without a local branch of the same name (`origin/x`).
    pub remote: Vec<String>,
}

pub fn branches(repo: &Path) -> Result<Branches, String> {
    let local: Vec<String> = run(repo, &["branch", "--format=%(refname:short)"])?.lines().map(str::to_owned).collect();
    let remote = run(repo, &["branch", "-r", "--format=%(refname:short)"])?
        .lines()
        .filter(|r| !r.ends_with("/HEAD") && r.contains('/'))
        .filter(|r| !local.iter().any(|l| r.split_once('/').is_some_and(|(_, b)| b == l)))
        .map(str::to_owned)
        .collect();
    Ok(Branches { local, remote })
}

/// `git push`, setting the upstream on the first push of a branch.
pub fn push(repo: &Path, st: &Status) -> Result<String, String> {
    if st.upstream.is_some() {
        return run(repo, &["push"]);
    }
    let remotes = run(repo, &["remote"])?;
    let remote = remotes.lines().find(|r| *r == "origin").or_else(|| remotes.lines().next());
    let Some(remote) = remote else { return Err("No remote configured for this repository".into()) };
    run(repo, &["push", "-u", remote, &st.branch])
}

/// Check out a local branch, or create a tracking branch for `origin/x`.
pub fn checkout(repo: &Path, branch: &str, remote: bool) -> Result<String, String> {
    if remote {
        let local = branch.split_once('/').map_or(branch, |(_, b)| b);
        run(repo, &["checkout", "-b", local, "--track", branch])
    } else {
        run(repo, &["checkout", branch])
    }
}

/// Undo work-tree changes; untracked files are deleted.
pub fn discard(repo: &Path, changes: &[Change]) -> Result<String, String> {
    let (untracked, tracked): (Vec<&Change>, Vec<&Change>) = changes.iter().partition(|c| c.untracked());
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--"];
        args.extend(tracked.iter().map(|c| c.path.as_str()));
        run(repo, &args)?;
    }
    for c in untracked {
        std::fs::remove_file(repo.join(&c.path)).map_err(|e| format!("{}: {e}", c.path))?;
    }
    Ok(String::new())
}

/// Diff of one file for display: staged or work-tree changes; untracked files in full.
pub fn diff(repo: &Path, c: &Change, staged: bool) -> Result<String, String> {
    if c.untracked() {
        // `--no-index` exits 1 when the files differ, which is always the case here.
        let out = Command::new("git")
            .args(["diff", "--no-index", "--", "/dev/null", &c.path])
            .current_dir(repo)
            .output()
            .map_err(|e| format!("Cannot run git: {e}"))?;
        return match out.status.code() {
            Some(0 | 1) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
            _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
        };
    }
    let mut args = vec!["diff"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", c.path.as_str()]);
    run(repo, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status() {
        let out =
            "## main...origin/main [ahead 2, behind 1]\0M  a.sql\0 M b.py\0R  new.sql\0old.sql\0?? c.yml\0AM d.md\0";
        let st = parse_status(out);
        assert_eq!(st.branch, "main");
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        let paths: Vec<&str> = st.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["a.sql", "b.py", "new.sql", "c.yml", "d.md"]);
        let staged: Vec<&str> = st.changes.iter().filter(|c| c.staged()).map(|c| c.path.as_str()).collect();
        assert_eq!(staged, ["a.sql", "new.sql", "d.md"]);
        let unstaged: Vec<&str> = st.changes.iter().filter(|c| c.unstaged()).map(|c| c.path.as_str()).collect();
        assert_eq!(unstaged, ["b.py", "c.yml", "d.md"]);
        assert_eq!(st.changes[3].letter(false), 'U');

        let st = parse_status("## No commits yet on main\0");
        assert_eq!((st.branch.as_str(), st.upstream.is_none()), ("main", true));
        let st = parse_status("## feature\0");
        assert_eq!((st.branch.as_str(), st.upstream.is_none()), ("feature", true));
        let st = parse_status("## x...origin/x [gone]\0");
        assert!(st.upstream.is_none());
    }

    #[test]
    fn real_repository() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ludo-git-{}", crate::db::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| run(&dir, args).unwrap();
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(dir.join("a.sql"), "select 1\n").unwrap();
        let st = status(&dir).unwrap();
        assert_eq!(st.branch, "main");
        assert!(st.changes[0].untracked());
        git(&["add", "-A"]);
        git(&["commit", "-qm", "init"]);
        std::fs::write(dir.join("a.sql"), "select 2\n").unwrap();
        std::fs::write(dir.join("b.sql"), "x").unwrap();
        let st = status(&dir).unwrap();
        assert!(diff(&dir, &st.changes[0], false).unwrap().contains("+select 2"));
        assert!(diff(&dir, &st.changes[1], false).unwrap().contains("+x"));
        discard(&dir, &st.changes).unwrap();
        assert!(status(&dir).unwrap().changes.is_empty());
        git(&["branch", "feature"]);
        assert_eq!(branches(&dir).unwrap().local, ["feature", "main"]);
        checkout(&dir, "feature", false).unwrap();
        assert_eq!(branch(&dir).as_deref(), Some("feature"));
        assert!(push(&dir, &status(&dir).unwrap()).unwrap_err().contains("No remote"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
