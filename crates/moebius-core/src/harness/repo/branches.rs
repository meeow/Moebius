//! The branches of the clone: proposals of the Retro and the branches of chats, checked and pushed when they are fit,
//! merged or dismissed by a person, and the throwaway worktree a trial runs a branch's agent in. The Retro's branches
//! that change only files under `moebius/` propose changes to Moebius's own configuration instead: they are read and
//! deleted, never pushed (see `improve::own`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::HarnessRepo;
use super::packages::PERSON_ONLY;
use super::read::{Layout, agent_config, broken_agents, read_agent_at};
use crate::app::{App, AppError, Result};
use crate::config::AgentConfig;
use crate::decide::{Actor, Question};
use crate::repos::{git, git_bytes};

impl HarnessRepo {
    /// The branches of the clone whose names start with `prefix` that are proposals: each is checked, and pushed to
    /// `origin` if it is fit. A branch that changes nothing, changes a file that is not harness configuration, breaks
    /// an agent that worked on the default branch, or cannot be pushed is left out, with a warning saying why.
    pub fn proposed(&self, app: &App, prefix: &str) -> Result<(Vec<Proposed>, Vec<String>)> {
        let _read_at = self.lock();
        let base = self.repos.default_branch(&self.path)?;
        let pattern = format!("refs/heads/{prefix}*");
        let branches = git(
            &self.path,
            &["for-each-ref", "--format=%(refname:short)", &pattern],
        )?;
        let broken_before = broken_agents(&self.path, &self.mcp_env);
        let (mut proposed, mut warnings) = (vec![], vec![]);
        for branch in branches.lines() {
            match self.check_branch(app, &base, branch, &broken_before) {
                Ok(p) => proposed.push(p),
                Err(why) => warnings.push(format!("{branch} is left out: {why}")),
            }
        }
        Ok((proposed, warnings))
    }

    /// The branches of the clone whose names start with `prefix` that change only files under `moebius/` (see
    /// `improve::own`): each is read and then deleted, since it is no harness configuration and is never pushed. A
    /// branch whose files cannot be read is left out, deleted too, with a warning saying why. Branches that change
    /// other files as well are left for `proposed`.
    pub fn take_own_branches(&self, prefix: &str) -> Result<(Vec<OwnBranch>, Vec<String>)> {
        let _read_at = self.lock();
        let base = self.repos.default_branch(&self.path)?;
        let pattern = format!("refs/heads/{prefix}*");
        let branches = git(
            &self.path,
            &["for-each-ref", "--format=%(refname:short)", &pattern],
        )?;
        let (mut taken, mut warnings) = (vec![], vec![]);
        for branch in branches.lines() {
            let range = format!("{base}...{branch}");
            let files = git(&self.path, &["diff", "--name-only", &range])?;
            let files: Vec<&str> = files.lines().collect();
            if files.is_empty() || !files.iter().all(|file| file.starts_with(OWN)) {
                continue;
            }
            match self.read_own_branch(branch, &files) {
                Ok(own) => taken.push(own),
                Err(why) => warnings.push(format!("{branch} is left out: {why}")),
            }
            git(&self.path, &["branch", "-D", "--quiet", branch])?;
        }
        Ok((taken, warnings))
    }

    fn read_own_branch(
        &self,
        branch: &str,
        files: &[&str],
    ) -> std::result::Result<OwnBranch, String> {
        let message =
            git(&self.path, &["log", "-1", "--format=%B", branch]).map_err(|e| e.to_string())?;
        let (message, cards) = split_cards(&message);
        let (title, body) = message.split_once('\n').unwrap_or((&message, ""));
        let mut texts = BTreeMap::new();
        for file in files {
            let bytes = git_bytes(&self.path, &["show", &format!("{branch}:{file}")])
                .map_err(|_| format!("it deletes {file}, which proposes nothing"))?;
            let text = String::from_utf8(bytes).map_err(|_| format!("{file} is not text"))?;
            texts.insert(file[OWN.len()..].to_string(), text);
        }
        Ok(OwnBranch {
            branch: branch.into(),
            title: title.trim().into(),
            body: body.trim().into(),
            cards,
            files: texts,
        })
    }

    /// Checks the branch of a chat that worked in the clone, the way a proposal is checked (see `proposed`): none when it
    /// left nothing beyond the default branch, and why it is not fit when it is not. A branch that is fit is pushed to
    /// `origin`.
    pub fn check_chat_branch(
        &self,
        app: &App,
        branch: &str,
    ) -> std::result::Result<Option<Proposed>, String> {
        let _read_at = self.lock();
        let base = self
            .repos
            .default_branch(&self.path)
            .map_err(|e| e.to_string())?;
        let beyond = format!("{base}..{branch}");
        let count =
            git(&self.path, &["rev-list", "--count", &beyond]).map_err(|e| e.to_string())?;
        if count == "0" {
            return Ok(None);
        }
        let broken_before = broken_agents(&self.path, &self.mcp_env);
        self.check_branch(app, &base, branch, &broken_before)
            .map(Some)
    }

    /// Creates the worktree of the chat `run_id`, on a new branch `moebius/chat-<id>` of the default branch.
    pub fn add_chat_worktree(&self, app: &App, run_id: i64) -> Result<crate::repos::Worktree> {
        let _read_at = self.lock();
        let base = self.repos.default_branch(&self.path)?;
        Ok(app
            .repos
            .add_chat_worktree("harness", &self.path, run_id, &base)?)
    }

    /// Makes the worktree of the chat `run_id` again on the branch it already has, after its agent went away and its
    /// worktree was removed with it.
    pub fn reuse_chat_worktree(&self, app: &App, run_id: i64) -> Result<crate::repos::Worktree> {
        let _read_at = self.lock();
        Ok(app
            .repos
            .reuse_chat_worktree("harness", &self.path, run_id)?)
    }

    fn check_branch(
        &self,
        app: &App,
        base: &str,
        branch: &str,
        broken_before: &BTreeMap<String, String>,
    ) -> std::result::Result<Proposed, String> {
        let git = |args: &[&str]| git(&self.path, args).map_err(|e| e.to_string());
        let range = format!("{base}...{branch}");
        let files = git(&["diff", "--name-only", &range])?;
        let files: Vec<&str> = files.lines().collect();
        if files.is_empty() {
            return Err("it changes nothing".into());
        }
        if let Some(file) = files.iter().find(|file| !is_configuration(file)) {
            return Err(format!(
                "it changes {file}, which is not harness configuration"
            ));
        }
        let check = format!("check-{}", branch.replace('/', "-"));
        let root = app
            .repos
            .add_detached_worktree("harness", &self.path, &check, branch)
            .map_err(|e| e.to_string())?;
        let broken = broken_agents(&root, &self.mcp_env);
        let _ = app.repos.remove_worktree(&self.path, &root);
        if let Some((name, error)) = broken
            .iter()
            .find(|(name, _)| !broken_before.contains_key(*name))
        {
            return Err(format!("the agent {name} cannot be used with it: {error}"));
        }
        let message = git(&["log", "-1", "--format=%B", branch])?;
        let (message, cards) = split_cards(&message);
        let (title, body) = message.split_once('\n').unwrap_or((&message, ""));
        let mut agents: Vec<String> = files
            .iter()
            .filter_map(|file| file.strip_prefix("agents/")?.rsplit_once('.'))
            .map(|(name, _)| name.to_string())
            .collect();
        agents.dedup();
        let header = app.git_auth(&self.url).map_err(|e| e.to_string())?;
        self.repos
            .push(&self.path, branch, header.as_deref())
            .map_err(|e| format!("it cannot be pushed: {e}"))?;
        Ok(Proposed {
            branch: branch.into(),
            title: title.trim().into(),
            body: body.trim().into(),
            agents,
            cards,
        })
    }

    /// A throwaway worktree of the harness configuration clone with `branch` checked out, named `name`, and the agent
    /// `agent` as the branch defines it, which is what a trial of the branch (see `trial`) runs. The agent's skills and
    /// extensions are files of the worktree, so the caller removes it (`remove_worktree`) once the run has ended.
    pub fn agent_on_branch(
        &self,
        app: &App,
        branch: &str,
        name: &str,
        agent: &str,
    ) -> Result<(AgentConfig, PathBuf)> {
        let _read_at = self.lock();
        let root = app
            .repos
            .add_detached_worktree("harness", &self.path, name, branch)?;
        let layout = Layout::read(&root, &self.mcp_env);
        match read_agent_at(&root, &layout, agent) {
            Ok((definition, mcp)) => Ok((agent_config(&root, &definition, mcp), root)),
            Err(why) => {
                let _ = app.repos.remove_worktree(&self.path, &root);
                Err(AppError::Invalid(why))
            }
        }
    }

    /// Removes a worktree made by `agent_on_branch`.
    pub fn remove_worktree(&self, app: &App, root: &Path) {
        let _ = app.repos.remove_worktree(&self.path, root);
    }

    /// A person merges the proposal `branch`: a merge commit on the default branch, pushed to `origin`. A merge that
    /// conflicts, and a push that fails, leave the clone as it was. On success the agents are read again and the
    /// person's decision is recorded; the merge commit's short id is given.
    pub fn merge_proposal(
        &self,
        app: &App,
        by: &Actor,
        branch: &str,
        title: &str,
        reason: &str,
    ) -> Result<String> {
        let message = format!("Merge the proposal: {title}");
        let head = self.merge_branch(app, by, branch, &message)?;
        self.record_proposal(app, by, branch, title, "merge_proposal", reason)?;
        Ok(git(&self.path, &["rev-parse", "--short", &head])?)
    }

    /// Merges `branch` into the default branch with a merge commit and pushes it to `origin`; a merge that conflicts,
    /// and a push that fails, leave the clone as it was. On success the agents are read again and the merge commit is
    /// given.
    ///
    /// The branch is read once, under the lock: what is checked and what is merged is the same commit. Anyone but a
    /// person is refused a branch that changes a package or the lock, since an extension is code.
    pub fn merge_branch(
        &self,
        app: &App,
        by: &Actor,
        branch: &str,
        message: &str,
    ) -> Result<String> {
        let mut read_at = self.lock();
        let before = git(&self.path, &["rev-parse", "HEAD"])?;
        let default = self.repos.default_branch(&self.path)?;
        let tip = git(
            &self.path,
            &["rev-parse", "--verify", &format!("{branch}^{{commit}}")],
        )?;
        if !matches!(by, Actor::Person) && self.changes_packages_at(&default, &tip)? {
            return Err(AppError::Forbidden(PERSON_ONLY.into()));
        }
        self.repos.merge(&self.path, &tip, message)?;
        let pushed = app
            .git_auth(&self.url)
            .and_then(|header| Ok(self.repos.push(&self.path, &default, header.as_deref())?));
        if let Err(e) = pushed {
            let _ = git(&self.path, &["reset", "--hard", "--quiet", &before]);
            return Err(e);
        }
        let head = git(&self.path, &["rev-parse", "HEAD"])?;
        self.read(app, &mut read_at, head.clone());
        Ok(head)
    }

    /// A person dismisses the proposal `branch`; nothing changes in the repository.
    pub fn dismiss_proposal(
        &self,
        app: &App,
        by: &Actor,
        branch: &str,
        title: &str,
        reason: &str,
    ) -> Result<()> {
        self.record_proposal(app, by, branch, title, "dismiss", reason)
    }

    fn record_proposal(
        &self,
        app: &App,
        by: &Actor,
        branch: &str,
        title: &str,
        choice: &str,
        reason: &str,
    ) -> Result<()> {
        app.decisions.record_by(
            by,
            &Question {
                kind: "merge_proposal".into(),
                context: BTreeMap::from([
                    ("branch".to_string(), branch.to_string()),
                    ("proposal".to_string(), title.to_string()),
                ]),
                options: vec!["merge_proposal".into(), "dismiss".into()],
                fail_closed: "dismiss".into(),
                run_id: None,
                card_id: None,
            },
            choice,
            reason,
        )?;
        Ok(())
    }
}

/// A branch of the harness configuration clone that is fit to be proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    pub branch: String,
    pub title: String,
    pub body: String,
    pub agents: Vec<String>,
    /// The cards named by the `Cards:` line that ends the commit message.
    pub cards: Vec<i64>,
}

/// The directory of a Retro branch that holds what it proposes for Moebius's own configuration.
const OWN: &str = "moebius/";

/// A branch of the Retro that proposes a change to Moebius's own configuration: its commit's message and the files it
/// holds under `moebius/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnBranch {
    pub branch: String,
    pub title: String,
    pub body: String,
    pub cards: Vec<i64>,
    /// Each file's path below `moebius/` and its text.
    pub files: BTreeMap<String, String>,
}

/// The commit message without its last line when that is `Cards: 48, 50`, and the numbers it names. Anything in the
/// list that is not a number is left out.
fn split_cards(message: &str) -> (String, Vec<i64>) {
    let message = message.trim_end();
    match message.rsplit_once('\n') {
        Some((rest, last)) if last.starts_with("Cards:") => {
            let cards = last["Cards:".len()..]
                .split([',', ' '])
                .filter_map(|n| n.parse().ok())
                .collect();
            (rest.trim_end().to_string(), cards)
        }
        _ => (message.to_string(), vec![]),
    }
}

/// Whether `file` is one of the files the harness configuration consists of.
fn is_configuration(file: &str) -> bool {
    ["agents/", "skills/", "extensions/", "packages/"]
        .iter()
        .any(|dir| file.starts_with(dir))
        || matches!(file, "rules.md" | "mcp.toml" | "packages.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cards_are_the_last_line_of_a_commit_message() {
        let (message, cards) = split_cards("Title\n\nBody.\n\nCards: 48, 50\n");
        assert_eq!((message.as_str(), cards), ("Title\n\nBody.", vec![48, 50]));
        let (message, cards) = split_cards("Title\n\nCards: 7,x, 9 ,");
        assert_eq!((message.as_str(), cards), ("Title", vec![7, 9]));
        // Only the last line counts, and a message without the line has no cards.
        let (message, cards) = split_cards("Title\n\nCards: 1\n\nMore.");
        assert_eq!(
            (message.as_str(), cards),
            ("Title\n\nCards: 1\n\nMore.", vec![])
        );
        let (message, cards) = split_cards("Title");
        assert_eq!((message.as_str(), cards), ("Title", vec![]));
    }
}
