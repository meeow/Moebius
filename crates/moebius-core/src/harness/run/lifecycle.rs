//! What a run holds from its start to its end, the same for every harness: the run recorded, its worktree and its
//! token (`card_run` for a card, `detached_run` for the Retro analyst and the Autoresearcher, `chat_run` for a chat,
//! `woken_chat` for a chat taken up again after its agent went away), what is done to undo it when its process cannot
//! be started (`launch_or_undo`), and `RunningRun`, which keeps it until the run ends (see `end`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use crate::app::{App, ChatSettings};
use crate::blobs::redact;
use crate::board::StartError;
use crate::config::{AgentConfig, HarnessKind};
use crate::harness::remote::{RemoteRun, Runners};
use crate::repos::{Worktree, git};
use crate::router::{Grant, Router};
use crate::store::{Card, Repository, Run};

/// The run's token, which the router knows the run by. The router checks the tool calls of the answers to a harness
/// without a bridge, Claude Code.
pub(in crate::harness) fn issue_token(
    router: &Router,
    config: &AgentConfig,
    grant: Grant,
) -> String {
    match config.harness {
        HarnessKind::PiBolt => router.issue_token(grant),
        HarnessKind::ClaudeCode => router.issue_checked_token(grant),
    }
}

/// Records a run of `agent` (on the harness `config` names) for `purpose`, with no card, and makes it a fresh detached
/// worktree of the clone `place` names (the name its worktrees go under, its path, and the repository the run records,
/// if any), checked out at `base`, with `files` written into it (see `write_untracked`), and its token. The run fails
/// when that cannot be done.
pub(in crate::harness) fn detached_run(
    app: &App,
    router: &Router,
    (agent, purpose): (&str, &str),
    (name, clone, repository): (&str, &Path, Option<i64>),
    (base, files): (&str, &BTreeMap<String, String>),
    config: &AgentConfig,
) -> Result<RunningRun, StartError> {
    let run = app
        .store
        .add_run(None, agent, config.harness.name(), purpose)
        .map_err(|e| StartError::Store(e.to_string()))?;
    let worktree = repository
        .map_or(Ok(()), |id| app.store.set_run_repository(run.id, id))
        .map_err(|e| e.to_string())
        .and_then(|()| {
            let worktree = format!("{purpose}-{}", run.id);
            app.repos
                .add_detached_worktree(name, clone, &worktree, base)
                .map_err(|e| e.to_string())
        })
        .and_then(|worktree| match write_untracked(&worktree, files) {
            Ok(()) => Ok(worktree),
            Err(e) => {
                let _ = app.repos.remove_worktree(clone, &worktree);
                Err(format!("cannot write the files of the run: {e}"))
            }
        })
        .map_err(|e| StartError::WorktreeFailed(fail(app, run.id, e)))?;
    let start = head(app, run.id, &worktree)?;
    let grant = Grant {
        run_id: run.id,
        card_id: None,
        budget: config.token_budget,
    };
    Ok(RunningRun {
        id: run.id,
        clone: Some(clone.to_path_buf()),
        start,
        worktree,
        token: issue_token(router, config, grant),
        purpose: purpose.into(),
        commit_message: String::new(),
        remote: None,
    })
}

/// Records a run of `card` by `agent`, as `config` has it (on the harness it names), for `purpose`, with the worktree
/// `worktree` makes for the run and its token. The agent leaves its work in the worktree, which is committed when the
/// run ends. The commit the worktree starts at is recorded with the run (`Store::set_run_base`), which the trials of the
/// regression set start from. The run fails when that cannot be done. When the agent names a runner, the run holds one
/// of its runs from here on (see `remote`), and no run is made when the runner is full.
pub(in crate::harness) fn card_run(
    app: &App,
    (router, runners): (&Router, &Arc<Runners>),
    card: &Card,
    (agent, config): (&str, &AgentConfig),
    purpose: &str,
    worktree: impl FnOnce(&Repository, &Path, i64) -> crate::repos::Result<Worktree>,
) -> Result<RunningRun, StartError> {
    let slot = config
        .runner
        .as_deref()
        .map(|name| runners.claim(name))
        .transpose()
        .map_err(StartError::RunnerFull)?;
    let repository = app
        .store
        .repository(card.repository_id)
        .map_err(|e| StartError::Store(e.to_string()))?
        .ok_or(StartError::RepositoryGone)?;
    let run = app
        .store
        .add_run(Some(card.id), agent, config.harness.name(), purpose)
        .map_err(|e| StartError::Store(e.to_string()))?;
    let clone = PathBuf::from(&repository.path);
    let worktree = worktree(&repository, &clone, run.id)
        .map_err(|e| StartError::WorktreeFailed(fail(app, run.id, e.to_string())))?;
    let start = head(app, run.id, &worktree.path)?;
    app.store
        .set_run_base(run.id, &start)
        .map_err(|e| StartError::Store(fail(app, run.id, e.to_string())))?;
    let grant = Grant {
        run_id: run.id,
        card_id: Some(card.id),
        budget: config.token_budget,
    };
    Ok(RunningRun {
        id: run.id,
        clone: Some(clone),
        start,
        worktree: worktree.path,
        token: issue_token(router, config, grant),
        purpose: purpose.to_string(),
        commit_message: format!(
            "#{}: {}\n\nCommitted by Moebius for its run.",
            card.number, card.title
        ),
        remote: slot.map(|slot| Arc::new(RemoteRun::new(slot, run.id))),
    })
}

/// What a started run holds until it ends, whatever its harness.
#[derive(Clone)]
pub(in crate::harness) struct RunningRun {
    pub(in crate::harness) id: i64,
    /// The repository's clone; none for a chat in an empty directory.
    pub(in crate::harness) clone: Option<PathBuf>,
    /// The commit the worktree was made at; what the run changed is everything after it.
    pub(in crate::harness) start: String,
    pub(in crate::harness) worktree: PathBuf,
    pub(in crate::harness) token: String,
    pub(in crate::harness) purpose: String,
    /// The message of the commit of what the agent leaves in the worktree.
    pub(in crate::harness) commit_message: String,
    /// The runner the harness goes on, if the run is not on the server.
    pub(in crate::harness) remote: Option<Arc<RemoteRun>>,
}

impl RunningRun {
    /// Commits what the agent left in the worktree of a repository; false when it changed nothing, that is, left
    /// the worktree clean and made no commit of its own. A chat without a repository has nothing to commit.
    pub(in crate::harness) fn commit(&self) -> crate::repos::Result<bool> {
        if self.clone.is_none() {
            return Ok(false);
        }
        let worktree = &self.worktree;
        if crate::repos::commit_all(worktree, &self.commit_message)? {
            return Ok(true);
        }
        Ok(git(worktree, &["rev-parse", "HEAD"])? != self.start)
    }

    /// Brings what the agent did on the runner into the worktree: committed there, fetched, and the worktree set to
    /// it. A run on the server has nothing to bring.
    pub(in crate::harness) fn bring_back(&self) -> Result<(), String> {
        match &self.remote {
            Some(remote) => remote.bring_back(&self.worktree, &self.commit_message),
            None => Ok(()),
        }
    }

    /// Closes the terminals open on the run, then removes the worktree, or the chat's empty directory, and what the
    /// run left on its runner.
    pub(in crate::harness) fn remove_worktree(&self, app: &App) {
        app.terminals.close(self.id);
        if let Some(remote) = &self.remote {
            remote.cleanup();
        }
        match &self.clone {
            Some(clone) => drop(app.repos.remove_worktree(clone, &self.worktree)),
            None => drop(std::fs::remove_dir_all(&self.worktree)),
        }
    }
}

/// Fails a run that could not be started, with an event saying why; returns the message.
pub(in crate::harness) fn fail(app: &App, run_id: i64, message: String) -> String {
    let _ = app
        .store
        .append_event(run_id, "moebius_error", &json!({ "message": message }));
    let _ = app.store.set_run_status(run_id, "failed");
    message
}

/// Writes `files` (paths relative to `worktree`, and their text) into the worktree and keeps them out of git: the first
/// directory of each path is added to the clone's `info/exclude`, which its worktrees share.
pub(super) fn write_untracked(
    worktree: &Path,
    files: &BTreeMap<String, String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let mut excluded = BTreeSet::new();
    for (path, text) in files {
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!("{path} is not a path inside the worktree"));
        }
        let file = worktree.join(relative);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        if let Some(first) = relative.components().next() {
            excluded.insert(format!("/{}", first.as_os_str().to_string_lossy()));
        }
    }
    let exclude =
        git(worktree, &["rev-parse", "--git-path", "info/exclude"]).map_err(|e| e.to_string())?;
    let exclude = worktree.join(exclude.trim());
    let mut text = std::fs::read_to_string(&exclude).unwrap_or_default();
    let missing: Vec<_> = excluded
        .iter()
        .filter(|pattern| !text.lines().any(|line| line == pattern.as_str()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for pattern in missing {
        text.push_str(pattern);
        text.push('\n');
    }
    if let Some(parent) = exclude.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&exclude, text).map_err(|e| e.to_string())
}

/// The commit `worktree` of the run `run_id` is at, which fails the run when git cannot say.
pub(in crate::harness) fn head(
    app: &App,
    run_id: i64,
    worktree: &Path,
) -> Result<String, StartError> {
    git(worktree, &["rev-parse", "HEAD"])
        .map_err(|e| StartError::WorktreeFailed(fail(app, run_id, e.to_string())))
}

/// Records a chat with `agent`, as `config` has it (on the harness it names), with `origin`, and makes its place: a
/// worktree on `moebius/chat-<id>` of `repository`, of the harness configuration repository when `harness` is set, or
/// an empty directory, and its token. The chat shows `message`, what the person said, and keeps the `settings` it was
/// given in place of its agent's. The chat fails when that cannot be done.
pub(in crate::harness) fn chat_run(
    app: &App,
    router: &Router,
    (agent, config): (&str, &AgentConfig),
    (repository, harness): (Option<&Repository>, bool),
    message: Option<&str>,
    origin: &str,
    settings: &ChatSettings,
) -> Result<RunningRun, StartError> {
    let run = app
        .store
        .add_run(None, agent, config.harness.name(), "chat")
        .map_err(|e| StartError::Store(e.to_string()))?;
    app.store
        .set_run_origin(run.id, origin)
        .map_err(|e| StartError::Store(fail(app, run.id, e.to_string())))?;
    let (clone, worktree, start) = match repository {
        None if harness => {
            let (made, clone) = app
                .start_harness_chat(run.id)
                .map_err(|e| StartError::WorktreeFailed(fail(app, run.id, e.to_string())))?;
            let start = head(app, run.id, &made.path)?;
            (Some(clone), made.path, start)
        }
        Some(repository) => {
            let clone = PathBuf::from(&repository.path);
            let made = app
                .store
                .set_run_repository(run.id, repository.id)
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    let base = repository.base_ref();
                    let name = &repository.name;
                    let worktree = app.repos.add_chat_worktree(name, &clone, run.id, &base);
                    worktree.map_err(|e| e.to_string())
                })
                .map_err(|e| StartError::WorktreeFailed(fail(app, run.id, e)))?;
            let start = head(app, run.id, &made.path)?;
            (Some(clone), made.path, start)
        }
        None => {
            let empty = app.data_dir.join("chats").join(run.id.to_string());
            std::fs::create_dir_all(&empty)
                .map_err(|e| StartError::WorktreeFailed(fail(app, run.id, e.to_string())))?;
            (None, empty, String::new())
        }
    };
    let grant = Grant {
        run_id: run.id,
        card_id: None,
        budget: config.token_budget,
    };
    let running = RunningRun {
        id: run.id,
        clone,
        start,
        worktree,
        token: issue_token(router, config, grant),
        purpose: "chat".into(),
        commit_message: format!(
            "Chat {}\n\nCommitted by Moebius when the chat ended.",
            run.id
        ),
        remote: None,
    };
    if let Some(message) = message {
        let said = json!({ "message": redact(message) });
        let _ = app.record_run_event(run.id, "said", &said);
    }
    if settings != &ChatSettings::default() {
        let _ = app.record_run_event(run.id, "settings_changed", &json!(settings));
    }
    Ok(running)
}

/// Makes again what the chat `stored` had before its agent went away: its worktree on the branch it already has, or the
/// empty directory that was removed with it, and a token of its own. The chat keeps its record, so what the person
/// writes next belongs to the chat they are reading; what its harness is told (`prompt`) carries what was said before.
/// A chat whose agent went on a runner is woken on the server, where its records are.
pub(in crate::harness) fn woken_chat(
    app: &App,
    router: &Router,
    stored: &Run,
    config: &AgentConfig,
) -> Result<RunningRun, String> {
    let id = stored.id;
    let (clone, worktree, start) = if let Some(repository) = stored.repository_id {
        let repository = app.repository(repository).map_err(|e| e.to_string())?;
        let clone = PathBuf::from(&repository.path);
        let made = app
            .repos
            .reuse_chat_worktree(&repository.name, &clone, id)
            .map_err(|e| e.to_string())?;
        let start = head(app, id, &made.path).map_err(|e| e.to_string())?;
        (Some(clone), made.path, start)
    } else if stored.harness_repository {
        let repo = app.harness_repo().map_err(|e| e.to_string())?;
        let made = repo
            .reuse_chat_worktree(app, id)
            .map_err(|e| e.to_string())?;
        let start = head(app, id, &made.path).map_err(|e| e.to_string())?;
        (Some(repo.path().to_path_buf()), made.path, start)
    } else {
        let empty = app.data_dir.join("chats").join(id.to_string());
        std::fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
        (None, empty, String::new())
    };
    let grant = Grant {
        run_id: id,
        card_id: None,
        budget: config.token_budget,
    };
    Ok(RunningRun {
        id,
        clone,
        start,
        worktree,
        token: issue_token(router, config, grant),
        purpose: "chat".into(),
        commit_message: format!("Chat {id}\n\nCommitted by Moebius when the chat ended."),
        remote: None,
    })
}

/// Starts the process of the run `running` with `launch` and returns the run; when the process cannot be started, what
/// was set up for the run is undone (its token revoked, its worktree removed) and the run has failed.
pub(in crate::harness) fn launch_or_undo(
    app: &App,
    router: &Router,
    running: RunningRun,
    launch: impl FnOnce(RunningRun) -> Result<(), String>,
) -> Result<Run, StartError> {
    if let Err(e) = launch(running.clone()) {
        // Nothing reads this run: undo what was set up for it.
        router.revoke_token(&running.token);
        running.remove_worktree(app);
        return Err(StartError::LaunchFailed(fail(app, running.id, e)));
    }
    app.store
        .run(running.id)
        .map_err(|e| StartError::Store(e.to_string()))
        .map(|run| run.expect("the run exists"))
}
