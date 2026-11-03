//! What the board engine, the chats and the improvement runs need of a harness, one trait for each caller: `CardRuns`
//! starts, steers and stops the runs of cards, `ChatRuns` serves the chats, and `ImproveRuns` starts the Retro, the
//! Autoresearcher and the trials. `Runs` is all three, which the server holds. Pi-Bolt (`pibolt`) and Claude Code
//! (`claude`) are the real ones, and `Harnesses` looks up which of them an agent runs on; the tests also use fakes.
//! No method has a default, so a harness that lacks one does not compile.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use crate::app::{App, ChatSettings};
use crate::config::{AgentConfig, HarnessKind};
use crate::router::Protocol;
use crate::store::{Card, Repository, Run};

/// An agent the harness can run, as the desktop API lists it: never its instructions or anything secret.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub model: String,
    pub protocol: Protocol,
    /// How hard the agent thinks, if its configuration says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// The most tokens a card of the agent may use, if its configuration says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    /// The harness the agent runs on.
    #[serde(default)]
    pub harness: HarnessKind,
    /// Whether Moebius brings the agent itself, as it does the Moebius agent.
    pub built_in: bool,
    /// Why the agent's files in the harness configuration repository could not be used. Such an agent has no model
    /// and cannot be run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The terminal interface of a chat while it is open: the program to run on a terminal. Dropped once the program has
/// ended, it gives the chat back to its own process (see [`ChatRuns::close_tui`]).
pub struct Tui {
    pub command: std::process::Command,
    runs: Arc<dyn ChatRuns>,
    app: Arc<App>,
    run_id: i64,
}

impl Tui {
    /// The terminal interface `command` that `runs` opened for the chat `run_id`.
    pub fn new(
        command: std::process::Command,
        runs: Arc<dyn ChatRuns>,
        app: Arc<App>,
        run_id: i64,
    ) -> Self {
        Self {
            command,
            runs,
            app,
            run_id,
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        self.runs.close_tui(&self.app, self.run_id);
    }
}

/// How one run of a card differs from the agent it is a run of: the model or model class it uses in place of the
/// agent's, and the runner it goes to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Variant<'a> {
    pub model: Option<&'a str>,
    pub runner: Option<&'a str>,
}

/// Why a run could not be started. Whoever starts runs fails closed on every cause: the card stays where it is.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The runner the run was to go on has no free run, or there is no such runner.
    #[error("{0}")]
    RunnerFull(String),
    /// The agent does not exist or cannot run on the harness.
    #[error("{0}")]
    UnknownAgent(String),
    /// The card's repository is gone.
    #[error("the card's repository is gone")]
    RepositoryGone,
    /// The run's worktree, or what is written into it, could not be made.
    #[error("{0}")]
    WorktreeFailed(String),
    /// The harness's process could not be started.
    #[error("{0}")]
    LaunchFailed(String),
    /// The run could not be recorded.
    #[error("{0}")]
    Store(String),
}

/// What the board engine needs of a harness: runs of cards. The real ones drive a harness (Pi-Bolt, Claude Code);
/// tests use fakes.
pub trait CardRuns: Send + Sync {
    /// Starts a run of `agent` for `card` and returns it without waiting for it to end. `variant` names the model and
    /// runner it uses in place of the agent's.
    fn start(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        purpose: (&str, Option<&str>),
        variant: Variant,
    ) -> Result<Run, StartError>;

    /// Whether an agent of this name is configured.
    fn has_agent(&self, name: &str) -> bool;

    /// The runners this harness can start runs on, with how many runs go on each.
    fn runners(&self) -> Vec<crate::harness::remote::RunnerInfo>;

    /// The agents this harness can run.
    fn agents(&self) -> Vec<AgentInfo>;

    /// The token budget of `agent` for a card, if it has one.
    fn token_budget(&self, agent: &str) -> Option<i64>;

    /// Sends a message to a running run's agent, which takes it into account as it works. False when the run is not
    /// running.
    fn steer(&self, run_id: i64, message: &str) -> bool;

    /// Asks a running run to stop; it ends as `stopped`. False when the run is not running.
    fn stop(&self, run_id: i64) -> bool;
}

/// What the chats need of a harness. A harness without a terminal interface refuses `tui`.
pub trait ChatRuns: Send + Sync {
    /// Moves the waiting chat `run_id` to its terminal interface and returns the program: the harness's interactive
    /// mode in the chat's worktree, set up as the chat's own process is and continuing the chat's conversation, whose
    /// process is ended meanwhile. What is said in the interface is recorded as the chat's events. A harness without
    /// one refuses.
    fn tui(&self, app: &Arc<App>, run_id: i64) -> Result<std::process::Command, String>;

    /// The terminal interface of the chat `run_id` has ended: the chat's own process starts again on the same
    /// conversation and waits for the next message, or, when the chat was ended or stopped meanwhile, the chat ends.
    fn close_tui(&self, app: &Arc<App>, run_id: i64);

    /// Starts a chat with `agent` in a branch of `repository`, of the harness configuration repository when `harness`
    /// is set, or in an empty directory, and returns it without waiting for the answer. The chat shows `message`, what
    /// the person wrote; the agent answers `prompt`, which is the message with what else the agent is told (the
    /// conversation an ended chat had, where the person is in the app). Without either the chat's agent starts and the
    /// chat waits for the first message. The chat is stored with `origin` (see `Run::origin`) and runs with `settings`
    /// in place of its agent's.
    #[allow(clippy::too_many_arguments)]
    fn start_chat(
        &self,
        app: &Arc<App>,
        agent: &str,
        place: (Option<&Repository>, bool),
        message: Option<&str>,
        prompt: Option<&str>,
        origin: &str,
        settings: &ChatSettings,
    ) -> Result<Run, StartError>;

    /// The waiting chat `run_id` was given other settings (see `ChatSettings`), which its latest `settings_changed`
    /// event holds: its agent answers the next message with them.
    fn change_chat(&self, app: &Arc<App>, run_id: i64) -> Result<(), String>;

    /// Sends a person's message to a chat's agent; it answers when it is done with what it is doing. False when the
    /// chat is not running.
    fn say(&self, run_id: i64, message: &str) -> bool;

    /// Starts the agent of the chat `run_id` again, after it went away and the chat ended as `stopped`: the same chat,
    /// on the branch and in the place it had, with `prompt` — what was said in the chat before, and what the person
    /// writes now. The chat works from now on; why its agent cannot start again is the error.
    fn wake(&self, app: &Arc<App>, run_id: i64, prompt: &str) -> Result<(), String>;

    /// Ends a chat: its agent is shut down and its work kept. False when the chat is not running.
    fn end(&self, run_id: i64) -> bool;
}

/// What the improvement runs need of a harness.
pub trait ImproveRuns: Send + Sync {
    /// Starts the Retro analyst with `prompt` in a fresh worktree of the harness configuration clone at `clone`,
    /// checked out at `base`, and returns the run without waiting for it to end. `files` (paths relative to the worktree,
    /// and their text) are written into the worktree before the analyst starts, and git leaves them out.
    fn start_retro(
        &self,
        app: &Arc<App>,
        clone: &Path,
        base: &str,
        prompt: &str,
        files: &BTreeMap<String, String>,
    ) -> Result<Run, StartError>;

    /// Starts the Autoresearcher (see `research`) with `prompt` in a fresh worktree of `repository` checked out at
    /// `base`, and returns the run without waiting for it to end.
    fn start_research(
        &self,
        app: &Arc<App>,
        repository: &Repository,
        base: &str,
        prompt: &str,
    ) -> Result<Run, StartError>;

    /// Starts a trial run (see `trial`) for `card`: the agent `agent`, as `definition` has it, is given the card's prompt
    /// in a fresh worktree of the card's repository on a branch of its own, checked out at `base`. The branch is named
    /// after `proposal` and the card. It returns the run without waiting for it to end.
    fn start_trial(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        definition: AgentConfig,
        proposal: i64,
        base: &str,
    ) -> Result<Run, StartError>;
}

/// All a harness does for the server: the three traits together.
pub trait Runs: CardRuns + ChatRuns + ImproveRuns {}

impl<T: CardRuns + ChatRuns + ImproveRuns> Runs for T {}
