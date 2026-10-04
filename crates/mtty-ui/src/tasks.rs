//! Agent tasks (B2.4): one git worktree and branch per task, so several
//! agents can work on one repository without touching each other's files.
//!
//! A task named `fix-login` in repository `R` lives in `R/.worktrees/fix-login`
//! on branch `mtty/fix-login` (the `.worktrees/<name>` layout the view rules
//! already recognise). The base branch is kept in the branch's git config, and
//! `.worktrees/` is added to the repository's local `.git/info/exclude`, so no
//! tracked file changes. Everything shells out to `git`.

use std::path::{Path, PathBuf};

/// Branch prefix for task branches.
pub const BRANCH_PREFIX: &str = "mtty/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub branch: String,
    pub path: PathBuf,
    /// The branch the task started from (merges go back into it).
    pub base: String,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = mtty_platform::background_command("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().next().unwrap_or("git failed").to_string())
    }
}

/// The repository's top level (main worktree) for any path inside it.
pub fn repo_root(dir: &Path) -> Result<PathBuf, String> {
    // From inside a task worktree, the main repository is the common dir's parent.
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common);
    Ok(common.parent().map(Path::to_path_buf).unwrap_or(common))
}

/// A task name usable as a directory and branch component.
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.len() <= 60
        && !name.starts_with(['-', '.'])
        && !name.ends_with(['.', '/'])
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Create a task from the repository's current branch.
pub fn create(dir: &Path, name: &str) -> Result<Task, String> {
    let name = name.trim();
    if !valid_name(name) {
        return Err("use letters, digits, '-', '_' or '.' for a task name".into());
    }
    let root = repo_root(dir)?;
    let base = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if base == "HEAD" {
        return Err("the repository is on a detached HEAD; check out a branch first".into());
    }
    let branch = format!("{BRANCH_PREFIX}{name}");
    let path = root.join(".worktrees").join(name);
    exclude_worktrees(&root)?;
    git(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &path.to_string_lossy(),
            "HEAD",
        ],
    )?;
    git(
        &root,
        &["config", &format!("branch.{branch}.mttyBase"), &base],
    )?;
    Ok(Task {
        name: name.to_string(),
        branch,
        path,
        base,
    })
}

/// Keep `.worktrees/` out of `git status` without touching tracked files.
fn exclude_worktrees(root: &Path) -> Result<(), String> {
    let info = PathBuf::from(git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", "info"],
    )?);
    std::fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let exclude = info.join("exclude");
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    if current.lines().any(|l| l.trim() == ".worktrees/") {
        return Ok(());
    }
    let sep = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    std::fs::write(&exclude, format!("{current}{sep}.worktrees/\n")).map_err(|e| e.to_string())
}

/// The repository's tasks.
pub fn list(dir: &Path) -> Result<Vec<Task>, String> {
    let root = repo_root(dir)?;
    let porcelain = git(&root, &["worktree", "list", "--porcelain"])?;
    let mut tasks = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in porcelain.lines().chain(std::iter::once("")) {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            if let (Some(name), Some(p)) = (b.strip_prefix(BRANCH_PREFIX), path.clone()) {
                let base = git(&root, &["config", &format!("branch.{b}.mttyBase")])
                    .unwrap_or_else(|_| "main".into());
                tasks.push(Task {
                    name: name.to_string(),
                    branch: b.to_string(),
                    path: p,
                    base,
                });
            }
        } else if line.is_empty() {
            path = None;
        }
    }
    Ok(tasks)
}

/// Everything the task changed relative to where it started, including
/// uncommitted edits in its worktree.
pub fn diff(task: &Task) -> Result<String, String> {
    let base = git(&task.path, &["merge-base", "HEAD", &task.base])?;
    let mut text = git(&task.path, &["diff", "--stat", &base])?;
    let patch = git(&task.path, &["diff", &base])?;
    let untracked = git(&task.path, &["ls-files", "--others", "--exclude-standard"])?;
    if !patch.is_empty() {
        text.push_str("\n\n");
        text.push_str(&patch);
    }
    if !untracked.is_empty() {
        text.push_str("\n\nUntracked files (not in the diff until added):\n");
        text.push_str(&untracked);
    }
    Ok(text)
}

fn is_clean(dir: &Path) -> Result<bool, String> {
    Ok(git(dir, &["status", "--porcelain"])?.is_empty())
}

/// Merge the task branch into its base branch in the main worktree. Both
/// worktrees must be clean: nothing uncommitted is merged or overwritten.
pub fn merge(task: &Task) -> Result<String, String> {
    let root = repo_root(&task.path)?;
    if !is_clean(&task.path)? {
        return Err("the task has uncommitted changes; commit them in its tab first".into());
    }
    if !is_clean(&root)? {
        return Err("the main worktree has uncommitted changes".into());
    }
    let current = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if current != task.base {
        return Err(format!(
            "the main worktree is on '{current}', not the task's base '{}'",
            task.base
        ));
    }
    git(
        &root,
        &[
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &format!("Merge task {}", task.name),
            &task.branch,
        ],
    )
    .map_err(|e| {
        let _ = git(&root, &["merge", "--abort"]);
        format!("merge failed and was aborted: {e}")
    })?;
    git(&root, &["log", "-1", "--format=%h %s"])
}

/// Remove the task's worktree and branch, discarding its work. Callers must
/// confirm with the user first.
pub fn discard(task: &Task) -> Result<(), String> {
    let root = repo_root(&task.path)?;
    git(
        &root,
        &[
            "worktree",
            "remove",
            "--force",
            &task.path.to_string_lossy(),
        ],
    )?;
    git(&root, &["branch", "-D", &task.branch])?;
    let _ = git(
        &root,
        &[
            "config",
            "--unset",
            &format!("branch.{}.mttyBase", task.branch),
        ],
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mtty-tasks-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "t@example.com"],
            &["config", "user.name", "t"],
            &["config", "commit.gpgsign", "false"],
            // Windows runners default to autocrlf, which would check the
            // merged file out with CRLF line endings.
            &["config", "core.autocrlf", "false"],
        ] {
            git(&dir, args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&dir, &["add", "."]).unwrap();
        git(&dir, &["commit", "-q", "-m", "init"]).unwrap();
        dir
    }

    #[test]
    fn names_are_safe_directory_and_branch_parts() {
        assert!(valid_name("fix-login_2"));
        for bad in ["", "-x", ".x", "a..b", "a/b", "a b", "x.", "中文"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_task_is_created_diffed_merged_and_listed() {
        let root = repo();
        let task = create(&root, "fix-login").unwrap();
        assert_eq!(task.branch, "mtty/fix-login");
        assert_eq!(task.base, "main");
        assert!(task.path.join("a.txt").is_file());
        assert!(is_clean(&root).unwrap(), ".worktrees/ is excluded locally");
        assert_eq!(
            list(&task.path).unwrap(),
            vec![task.clone()],
            "found from inside the task too"
        );

        std::fs::write(task.path.join("a.txt"), "one\ntwo\n").unwrap();
        let d = diff(&task).unwrap();
        assert!(d.contains("+two"), "{d}");
        assert!(merge(&task).unwrap_err().contains("uncommitted"));

        git(&task.path, &["commit", "-qam", "add two"]).unwrap();
        let merged = merge(&task).unwrap();
        assert!(merged.contains("Merge task fix-login"), "{merged}");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "one\ntwo\n"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn discarding_removes_the_worktree_and_branch() {
        let root = repo();
        let task = create(&root, "spike").unwrap();
        std::fs::write(task.path.join("b.txt"), "draft\n").unwrap();
        discard(&task).unwrap();
        assert!(!task.path.exists());
        assert!(list(&root).unwrap().is_empty());
        assert!(git(&root, &["rev-parse", "--verify", "mtty/spike"]).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "one\n"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn merge_refuses_a_dirty_main_worktree_or_another_branch() {
        let root = repo();
        let task = create(&root, "t1").unwrap();
        std::fs::write(task.path.join("c.txt"), "x\n").unwrap();
        git(&task.path, &["add", "."]).unwrap();
        git(&task.path, &["commit", "-qm", "c"]).unwrap();
        std::fs::write(root.join("a.txt"), "local edit\n").unwrap();
        assert!(merge(&task).unwrap_err().contains("main worktree"));
        git(&root, &["checkout", "-q", "--", "a.txt"]).unwrap();
        git(&root, &["checkout", "-q", "-b", "other"]).unwrap();
        assert!(merge(&task).unwrap_err().contains("not the task's base"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
