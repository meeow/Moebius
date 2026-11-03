//! Repositories and worktrees (spec 3): one clone per repository under the data directory, a worktree and branch per
//! run, and local merges for repositories without a forge. Everything goes through the `git` command line.

pub mod files;
pub mod forge;
pub mod forgejo;
pub mod github;
pub mod gitlab;
pub mod poll;
pub mod search;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Why a git operation failed.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("cannot run git: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("git {args} failed: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("{branch} does not merge cleanly into {into}")]
    Conflict { branch: String, into: String },
}

pub type Result<T> = std::result::Result<T, GitError>;

/// The name Moebius commits and merges under.
const IDENTITY: [&str; 4] = [
    "-c",
    "user.name=Moebius",
    "-c",
    "user.email=moebius@localhost",
];

/// Runs git in `dir` and returns its trimmed standard output.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    git_with_config(dir, args, &[])
}

/// Runs git in `dir` and returns its standard output as it is, for what may not be text.
pub fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_output(dir, args, &[])?;
    if !output.status.success() {
        return Err(GitError::Failed {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output.stdout)
}

/// Runs git in `dir` with the extra environment variables `env` and returns its standard output as it is.
pub fn git_bytes_env(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(IDENTITY)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().copied())
        .output()?;
    if !output.status.success() {
        return Err(GitError::Failed {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output.stdout)
}

/// Runs git in `dir` with `input` on its standard input and returns its standard output as it is, for `cat-file --batch`.
pub fn git_with_input(dir: &Path, args: &[&str], input: Vec<u8>) -> Result<Vec<u8>> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("git")
        .args(IDENTITY)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Written on a thread of its own, so that git never waits for us to read while we wait for it to read.
    let mut stdin = child.stdin.take().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "git has no standard input")
    })?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output()?;
    let _ = writer.join();
    if !output.status.success() {
        return Err(GitError::Failed {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output.stdout)
}

/// Commits everything in the worktree `worktree`, new files too, with `message`; false when there was nothing to commit.
pub fn commit_all(worktree: &Path, message: &str) -> Result<bool> {
    git(worktree, &["add", "--all"])?;
    if git(worktree, &["status", "--porcelain"])?.is_empty() {
        return Ok(false);
    }
    git(worktree, &["commit", "--quiet", "-m", message])?;
    Ok(true)
}

/// Runs git in `dir` with the configuration `config` (name and value) for this command only. It is handed over in
/// the environment, so a credential in it shows up neither in `.git/config`, nor in an error message.
fn git_with_config(dir: &Path, args: &[&str], config: &[(&str, &str)]) -> Result<String> {
    let output = git_output(dir, args, config)?;
    if !output.status.success() {
        return Err(GitError::Failed {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Runs git as `git_with_config` does and returns what it printed, whatever its exit status.
fn git_output(
    dir: &Path,
    args: &[&str],
    config: &[(&str, &str)],
) -> std::io::Result<std::process::Output> {
    let mut command = Command::new("git");
    command
        .args(IDENTITY)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", config.len().to_string());
    for (i, (name, value)) in config.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{i}"), name)
            .env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
    command.output()
}

/// The configuration that adds `auth` as an HTTP header to one git command.
fn auth_config(auth: Option<&str>) -> Vec<(&str, &str)> {
    auth.map(|header| ("http.extraHeader", header))
        .into_iter()
        .collect()
}

/// The name of the worktrees and the branch of the trial of proposal `proposal` on card `card`.
pub fn trial_name(proposal: i64, card: i64) -> String {
    format!("trial-{proposal}-card-{card}")
}

/// A worktree made for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

/// The clones and worktrees under the data directory.
#[derive(Debug, Clone)]
pub struct Repos {
    clones: PathBuf,
    worktrees: PathBuf,
    /// Held while worktrees are added or removed: git cannot add one while another is pruned or added beside it.
    worktree_lock: std::sync::Arc<std::sync::Mutex<()>>,
    /// One lock per clone, held while something changes its checked-out default branch.
    clone_locks: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
    >,
}

impl Repos {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            clones: data_dir.join("repos"),
            worktrees: data_dir.join("worktrees"),
            worktree_lock: Default::default(),
            clone_locks: Default::default(),
        }
    }

    /// The lock of the clone at `clone`: whoever changes the clone's checked-out default branch (a file saved, a
    /// change merged, a fast-forward) holds it, so that two of them never meet in one working tree.
    pub(crate) fn clone_lock(&self, clone: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
        let mut locks = self.clone_locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(clone.to_path_buf()).or_default().clone()
    }

    /// Keeps other threads from adding or removing worktrees until it is dropped.
    fn hold_worktrees(&self) -> std::sync::MutexGuard<'_, ()> {
        self.worktree_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Where the clone of the repository `name` lives.
    pub fn clone_path(&self, name: &str) -> PathBuf {
        self.clones.join(name)
    }

    /// Deletes the clone of the repository `name` and every worktree under it, so that a removed repository leaves
    /// nothing on disk behind.
    pub fn remove(&self, name: &str) -> Result<()> {
        let _held = self.hold_worktrees();
        for dir in [self.worktrees.join(name), self.clone_path(name)] {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
        }
        Ok(())
    }

    /// Clones `url` as `name`, or fetches if the clone already exists and came from `url`. A leftover clone of
    /// another origin (a removed repository whose files could not all be deleted) is deleted and cloned afresh.
    /// Returns the clone's path.
    pub fn clone_or_fetch(&self, name: &str, url: &str, auth: Option<&str>) -> Result<PathBuf> {
        let path = self.clone_path(name);
        let same_origin =
            path.join(".git").exists() && self.origin_url(&path).is_ok_and(|origin| origin == url);
        if same_origin {
            self.fetch(&path, auth)?;
        } else {
            self.remove(name)?;
            self.clone_into(&path, url, auth)?;
        }
        Ok(path)
    }

    /// Makes `path` a new repository with one empty commit on `main`, creating the folder if it does not exist.
    pub fn init(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)?;
        git(path, &["init", "--quiet", "--initial-branch=main"])?;
        git(
            path,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "Start the repository",
            ],
        )
        .map(drop)
    }

    /// Clones `url` to `path`, with `auth` as an extra HTTP header if the remote needs one.
    pub fn clone_into(&self, path: &Path, url: &str, auth: Option<&str>) -> Result<()> {
        let parent = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let target = path.to_string_lossy().into_owned();
        let args = ["clone", "--quiet", "--", url, &target];
        git_with_config(parent, &args, &auth_config(auth)).map(drop)
    }

    /// Fetches `origin`, with `auth` as an extra HTTP header if the remote needs one.
    pub fn fetch(&self, clone: &Path, auth: Option<&str>) -> Result<()> {
        let args = ["fetch", "--quiet", "--prune", "origin"];
        git_with_config(clone, &args, &auth_config(auth)).map(drop)
    }

    /// The URL of the clone's remote `origin`.
    pub fn origin_url(&self, clone: &Path) -> Result<String> {
        git(clone, &["remote", "get-url", "origin"])
    }

    /// Pushes `branch` to `origin`, with `auth` as an extra HTTP header if the remote needs one.
    pub fn push(&self, clone: &Path, branch: &str, auth: Option<&str>) -> Result<()> {
        let args = ["push", "--quiet", "origin", branch];
        git_with_config(clone, &args, &auth_config(auth)).map(drop)
    }

    /// The branch checked out in the clone, which Moebius treats as its default branch.
    pub fn default_branch(&self, clone: &Path) -> Result<String> {
        git(clone, &["symbolic-ref", "--short", "HEAD"])
    }

    /// Creates a worktree for run `run_id` on a new branch `moebius/run-<id>` starting at `base`.
    pub fn add_worktree(
        &self,
        name: &str,
        clone: &Path,
        run_id: i64,
        base: &str,
    ) -> Result<Worktree> {
        self.add_named_worktree(name, clone, &format!("run-{run_id}"), base)
    }

    /// Creates a worktree for chat `run_id` on a new branch `moebius/chat-<id>` starting at `base`.
    pub fn add_chat_worktree(
        &self,
        name: &str,
        clone: &Path,
        run_id: i64,
        base: &str,
    ) -> Result<Worktree> {
        self.add_named_worktree(name, clone, &format!("chat-{run_id}"), base)
    }

    /// Creates a worktree for the trial of proposal `proposal` on card `card` on a new branch
    /// `moebius/trial-<proposal>-card-<card>` starting at `base`.
    pub fn add_trial_worktree(
        &self,
        repository: &str,
        clone: &Path,
        proposal: i64,
        card: i64,
        base: &str,
    ) -> Result<Worktree> {
        self.add_named_worktree(repository, clone, &trial_name(proposal, card), base)
    }

    /// Deletes the branch of a trial, once its worktree is gone.
    pub fn remove_trial_branch(&self, clone: &Path, proposal: i64, card: i64) -> Result<()> {
        let branch = format!("moebius/{}", trial_name(proposal, card));
        git(clone, &["branch", "-D", "--quiet", &branch]).map(drop)
    }

    /// Creates the worktree of the chat `run_id` again, on the branch `moebius/chat-<id>` it already has, once the
    /// worktree that went with it was removed when its agent went away. The branch is taken as it stands, so what the
    /// chat left behind is back where its agent expects it.
    pub fn reuse_chat_worktree(&self, name: &str, clone: &Path, run_id: i64) -> Result<Worktree> {
        let branch = format!("moebius/chat-{run_id}");
        let path = self.worktrees.join(name).join(format!("chat-{run_id}"));
        std::fs::create_dir_all(path.parent().expect("worktrees have a parent"))?;
        let target = path.to_string_lossy().into_owned();
        let _held = self.hold_worktrees();
        git(clone, &["worktree", "add", "--quiet", &target, &branch])?;
        Ok(Worktree { path, branch })
    }

    fn add_named_worktree(
        &self,
        repository: &str,
        clone: &Path,
        name: &str,
        base: &str,
    ) -> Result<Worktree> {
        let branch = format!("moebius/{name}");
        let path = self.worktrees.join(repository).join(name);
        std::fs::create_dir_all(path.parent().expect("worktrees have a parent"))?;
        let target = path.to_string_lossy().into_owned();
        let _held = self.hold_worktrees();
        git(
            clone,
            &["worktree", "add", "--quiet", "-b", &branch, &target, base],
        )?;
        Ok(Worktree { path, branch })
    }

    /// Creates a throwaway worktree named `name` with `rev` checked out and no branch. A leftover of the same name is
    /// removed first.
    pub fn add_detached_worktree(
        &self,
        repository: &str,
        clone: &Path,
        name: &str,
        rev: &str,
    ) -> Result<PathBuf> {
        let path = self.worktrees.join(repository).join(name);
        std::fs::create_dir_all(path.parent().expect("worktrees have a parent"))?;
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        let _held = self.hold_worktrees();
        git(clone, &["worktree", "prune"])?;
        let target = path.to_string_lossy().into_owned();
        git(
            clone,
            &["worktree", "add", "--quiet", "--detach", &target, rev],
        )?;
        Ok(path)
    }

    /// Removes a worktree; the branch of a run's worktree is kept, since the change lives on it.
    pub fn remove_worktree(&self, clone: &Path, worktree: &Path) -> Result<()> {
        let target = worktree.to_string_lossy().into_owned();
        let _held = self.hold_worktrees();
        git(clone, &["worktree", "remove", "--force", &target])?;
        git(clone, &["worktree", "prune"]).map(drop)
    }

    /// Points `branch` at `rev`, creating it if need be.
    pub fn point_branch(&self, clone: &Path, branch: &str, rev: &str) -> Result<()> {
        git(clone, &["branch", "--force", branch, rev]).map(drop)
    }

    /// The files that conflict when `branch` is merged into `base`, one per line; none if it merges cleanly. Nothing
    /// is checked out or changed.
    pub fn conflicts(&self, clone: &Path, base: &str, branch: &str) -> Result<Option<String>> {
        let args = ["merge-tree", "--write-tree", "--name-only", base, branch];
        let output = git_output(clone, &args, &[])?;
        let text = String::from_utf8_lossy(&output.stdout);
        match output.status.code() {
            Some(0) => Ok(None),
            // The first line is the tree; the conflicting files follow up to the first empty line. Without a tree, git
            // could not merge at all, such as for a branch that does not exist.
            Some(1) if !text.is_empty() => Ok(Some(
                text.lines()
                    .skip(1)
                    .take_while(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )),
            _ => Err(GitError::Failed {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }

    /// Merges `branch` into the default branch checked out in the clone and returns the merge commit. A conflict
    /// leaves the default branch as it was.
    pub fn merge(&self, clone: &Path, branch: &str, message: &str) -> Result<String> {
        let lock = self.clone_lock(clone);
        let _merging = lock.lock().unwrap_or_else(|e| e.into_inner());
        let into = self.default_branch(clone)?;
        match git(
            clone,
            &["merge", "--no-ff", "--quiet", "-m", message, branch],
        ) {
            Ok(_) => git(clone, &["rev-parse", "HEAD"]),
            Err(GitError::Failed { .. }) => {
                let _ = git(clone, &["merge", "--abort"]);
                Err(GitError::Conflict {
                    branch: branch.to_string(),
                    into,
                })
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An origin repository with one commit on `main`.
    fn origin(dir: &Path) -> PathBuf {
        let origin = dir.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(origin.join("README.md"), "hello\n").unwrap();
        git(&origin, &["add", "."]).unwrap();
        git(&origin, &["commit", "--quiet", "-m", "first"]).unwrap();
        origin
    }

    fn commit(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        git(dir, &["add", "."]).unwrap();
        git(dir, &["commit", "--quiet", "-m", file]).unwrap();
    }

    #[test]
    fn throwaway_worktrees_made_and_removed_at_once_on_many_threads_all_work() {
        let dir = tempfile::tempdir().unwrap();
        let origin = origin(dir.path());
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (repos, clone) = (repos.clone(), clone.clone());
                std::thread::spawn(move || {
                    for round in 0..3 {
                        let name = format!("throwaway-{i}-{round}");
                        let worktree = repos
                            .add_detached_worktree("demo", &clone, &name, "main")
                            .unwrap();
                        assert!(worktree.join("README.md").exists());
                        repos.remove_worktree(&clone, &worktree).unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn a_leftover_clone_of_another_origin_is_cloned_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let first = origin(dir.path());
        let second = dir.path().join("second");
        std::fs::create_dir(&second).unwrap();
        git(&second, &["init", "--quiet", "--initial-branch=main"]).unwrap();
        commit(&second, "OTHER.md", "other");
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", first.to_str().unwrap(), None)
            .unwrap();
        assert!(clone.join("README.md").exists());

        let again = repos
            .clone_or_fetch("demo", second.to_str().unwrap(), None)
            .unwrap();

        assert_eq!(
            repos.origin_url(&again).unwrap(),
            second.to_str().unwrap(),
            "the clone follows the new URL"
        );
        assert!(again.join("OTHER.md").exists());
        assert!(!again.join("README.md").exists());
    }

    #[test]
    fn worktrees_are_created_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let origin = origin(dir.path());
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(repos.default_branch(&clone).unwrap(), "main");
        // A second call fetches instead of cloning.
        repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();

        let wt = repos.add_worktree("demo", &clone, 7, "main").unwrap();
        assert_eq!(wt.branch, "moebius/run-7");
        assert!(wt.path.join("README.md").is_file());
        repos.remove_worktree(&clone, &wt.path).unwrap();
        assert!(!wt.path.exists());
        assert!(git(&clone, &["rev-parse", "--verify", "moebius/run-7"]).is_ok());
    }

    #[test]
    fn a_branch_is_pushed_to_origin() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.git");
        git(
            dir.path(),
            &["init", "--quiet", "--bare", "-b", "main", "bare.git"],
        )
        .unwrap();
        let origin = origin(dir.path());
        git(
            &origin,
            &["push", "--quiet", bare.to_str().unwrap(), "main"],
        )
        .unwrap();
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", bare.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(repos.origin_url(&clone).unwrap(), bare.to_str().unwrap());
        let wt = repos.add_worktree("demo", &clone, 3, "main").unwrap();
        commit(&wt.path, "feature.txt", "new\n");
        repos.remove_worktree(&clone, &wt.path).unwrap();

        repos.push(&clone, &wt.branch, None).unwrap();
        assert!(git(&bare, &["rev-parse", "--verify", "moebius/run-3"]).is_ok());
        // A header given to a push is not kept.
        repos
            .push(&clone, &wt.branch, Some("Authorization: secret"))
            .unwrap();
        let config = std::fs::read_to_string(clone.join(".git/config")).unwrap();
        assert!(!config.contains("secret"));
    }

    #[test]
    fn a_local_merge_lands_on_the_default_branch() {
        let dir = tempfile::tempdir().unwrap();
        let origin = origin(dir.path());
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();
        let wt = repos.add_worktree("demo", &clone, 1, "main").unwrap();
        commit(&wt.path, "feature.txt", "new\n");
        repos.remove_worktree(&clone, &wt.path).unwrap();

        let sha = repos.merge(&clone, &wt.branch, "Merge run 1").unwrap();
        assert_eq!(git(&clone, &["rev-parse", "main"]).unwrap(), sha);
        assert!(clone.join("feature.txt").is_file());
    }

    #[test]
    fn a_conflicting_merge_is_refused_and_leaves_the_branch_alone() {
        let dir = tempfile::tempdir().unwrap();
        let origin = origin(dir.path());
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();
        let wt = repos.add_worktree("demo", &clone, 2, "main").unwrap();
        commit(&wt.path, "README.md", "theirs\n");
        commit(&clone, "README.md", "ours\n");
        let before = git(&clone, &["rev-parse", "main"]).unwrap();
        assert!(matches!(
            repos.merge(&clone, &wt.branch, "Merge run 2"),
            Err(GitError::Conflict { .. })
        ));
        assert_eq!(git(&clone, &["rev-parse", "main"]).unwrap(), before);
        assert_eq!(git(&clone, &["status", "--porcelain"]).unwrap(), "");
    }

    #[test]
    fn conflicts_are_found_without_touching_anything() {
        let dir = tempfile::tempdir().unwrap();
        let origin = origin(dir.path());
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();
        let wt = repos.add_worktree("demo", &clone, 4, "main").unwrap();
        commit(&wt.path, "feature.txt", "new\n");
        commit(&wt.path, "README.md", "theirs\n");
        repos.remove_worktree(&clone, &wt.path).unwrap();
        assert_eq!(repos.conflicts(&clone, "main", &wt.branch).unwrap(), None);

        commit(&clone, "README.md", "ours\n");
        commit(&clone, "feature.txt", "ours too\n");
        let before = git(&clone, &["rev-parse", "main"]).unwrap();
        assert_eq!(
            repos.conflicts(&clone, "main", &wt.branch).unwrap(),
            Some("README.md\nfeature.txt".into())
        );
        assert_eq!(git(&clone, &["rev-parse", "main"]).unwrap(), before);
        assert_eq!(git(&clone, &["status", "--porcelain"]).unwrap(), "");
        assert!(repos.conflicts(&clone, "main", "nonexistent").is_err());
    }

    #[test]
    fn a_branch_is_moved_forward_and_a_commit_is_checked_out_in_a_throwaway_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let origin = origin(dir.path());
        let repos = Repos::new(&dir.path().join("data"));
        let clone = repos
            .clone_or_fetch("demo", origin.to_str().unwrap(), None)
            .unwrap();
        let wt = repos.add_worktree("demo", &clone, 5, "main").unwrap();
        commit(&wt.path, "feature.txt", "new\n");
        repos.remove_worktree(&clone, &wt.path).unwrap();

        repos
            .point_branch(&clone, "moebius/card-1", &wt.branch)
            .unwrap();
        let head = git(&clone, &["rev-parse", "moebius/card-1"]).unwrap();
        assert_eq!(git(&clone, &["rev-parse", &wt.branch]).unwrap(), head);

        let checkout = repos
            .add_detached_worktree("demo", &clone, "checks-1", &head)
            .unwrap();
        assert!(checkout.join("feature.txt").is_file());
        // A leftover of an earlier attempt is replaced.
        let again = repos
            .add_detached_worktree("demo", &clone, "checks-1", &head)
            .unwrap();
        repos.remove_worktree(&clone, &again).unwrap();
        assert!(!again.exists());
    }
}
