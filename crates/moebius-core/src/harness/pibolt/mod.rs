//! The Pi-Bolt harness (spec 4): each run is `pi-bolt --mode rpc` in the run's worktree, with the router as its only
//! provider. The prompt goes in as one JSON line; every record Pi writes is stored as a run event and published on the
//! bus. Records are split on LF only, never on U+2028 or U+2029, which are valid inside JSON strings. The run ends at
//! `agent_settled`; what the agent changed is committed on the run's branch and the worktree is removed. A chat is
//! the exception: its Pi stays up at `agent_settled`, the run waits for the person's next message, and it ends when
//! the person ends or stops it (or Pi exits). A chat works in a worktree on `moebius/chat-<id>` or, without a
//! repository, in an empty directory; what it left is committed on the branch when it ends, and when the branch
//! then has commits beyond the default branch they are offered as a change without a card. A `code` run
//! that changed nothing, that is, left no change and made no commit, has failed; other runs, such as a review, are not committed. A run starts from the branch of the
//! card's latest successful `code` run, so it sees the work so far. The text of the agent's final answer is kept as
//! the run's summary. A run is stopped with `abort` and steered with `steer`. Pi runs offline and does not trust
//! configuration found in the worktree, and loads only the extensions, skills and MCP servers the agent is configured with (see `mcp`). The
//! Moebius bridge (`bridge/moebius-bridge.ts`) is loaded into every run after the agent's own extensions, so that it
//! sees the final input and content of each tool call; it asks the check for each tool call and result (see `check`). Every run is told the path of the
//! `moebius` executable as `$MOEBIUS`. The Moebius agent is always available: its instructions are built in, its skills
//! are the guide and the task skills (see `skill`), and its model comes from `[agents.Moebius]` or else from the Coder. The Retro analyst
//! (see `retro`) is built the same way; its run works in a worktree of the harness configuration repository. So is the
//! Autoresearcher (see `research`), whose run works in a worktree of the repository it researches. A person can also
//! open a waiting chat in Pi-Bolt's own interactive mode (`tui`), and it stays one conversation: the chat's idle Pi is
//! ended, Pi's interactive mode continues the chat's session (`--continue`, the same agent directory, sessions and run
//! token), and what it adds to the session file is recorded as the chat's events (see `session_events`). When the
//! interface closes, the chat's Pi starts again with `--continue` and waits for the next message. A chat can run on
//! another model and reasoning level than its agent's (see `ChatSettings`), with which its Pi is started every time; a
//! waiting chat given another reasoning level is told it with `set_thinking_level`, and one given another model gets a
//! new Pi on the same session, since Pi knows only the models of its `models.json`.
//!
//! `process` writes Pi's agent directory (`models.json`, `mcp.json`), builds its command line and starts it; `reader`
//! reads Pi's records and ends the run; `chat` keeps a chat's Pi going between messages, with its settings, and pauses
//! an idle one; `tui` hands a waiting chat to Pi's interactive mode and records what its session file gains. Making the
//! run, its worktree and its token, its prompts, the built-in agents and ending the run are the same for every
//! harness and live in `run`.

mod chat;
// Unix only: the fake Pi and the fake launcher are Python scripts started by their `#!` lines.
#[cfg(all(test, unix))]
mod launcher_tests;
mod process;
mod reader;
// Unix only: the fake `ssh` and the fake Pi are Python scripts started by their `#!` lines.
#[cfg(all(test, unix))]
mod remote_tests;
#[cfg(all(test, unix))]
pub(in crate::harness) mod testing;
mod tui;

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::{App, ChatSettings};
use crate::board::{AgentInfo, CardRuns, ChatRuns, ImproveRuns, StartError, Variant};
use crate::config::{AgentConfig, AgentSet};
use crate::harness::launch::{Launched, Launcher};
use crate::harness::remote::Runners;
use crate::router::Router;
use crate::store::{Card, Repository, Run};
use tui::Follower;

pub use crate::harness::run::{
    AUTORESEARCHER_AGENT, BuiltInAgent, GRILLING_AGENT, MOEBIUS_AGENT, RETRO_AGENT, built_in_agent,
};
use crate::harness::run::{
    Launching, RunningRun, launch_or_undo, start_card, start_detached, start_trial, woken_chat,
};
pub(crate) use crate::harness::run::{
    agent_list, chat_agent, final_answer, has_agent, resolve_agent, tell_where_moebius_runs,
};

/// How long Pi may take to exit after it was asked to stop.
const STOP_GRACE: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(10)
};

/// How long a chat waits for the person's next message before its Pi is ended, to be started again on the same session
/// by the next message (see `PiBolt::pause_idle_chats`).
pub const IDLE: Duration = Duration::from_secs(30 * 60);

/// The bridge extension, written into each run's directory.
const BRIDGE: &str = include_str!("../../../bridge/moebius-bridge.ts");

/// The lens extension, written into each run's directory next to the bridge.
use crate::harness::lens::EXTENSION as LENS;

/// Starts and stops Pi-Bolt runs.
pub struct PiBolt {
    /// The `pi-bolt` executable.
    pub program: PathBuf,
    /// The `moebius` executable, which runs find as `$MOEBIUS`.
    pub moebius: PathBuf,
    /// The router's base URL, such as `http://127.0.0.1:7172`.
    pub router_url: String,
    /// The desktop API's base URL, which a run of the Moebius agent is told as `MOEBIUS_URL`; set once the server
    /// listens, since the address is only known then.
    pub api_url: OnceLock<String>,
    pub router: Arc<Router>,
    /// The agents runs use. The harness configuration repository swaps them when it changes.
    pub agents: Arc<RwLock<AgentSet>>,
    /// The machines an agent's `runner` can name; none unless the server sets them.
    pub runners: Arc<Runners>,
    /// The launcher plugin that may wrap how Pi starts, such as in a sandbox; none unless `[[plugins]]` names one.
    /// With one, no Pi starts without asking it, and one that cannot be asked does not start.
    pub launcher: Option<Arc<Launcher>>,
    live: Arc<Mutex<HashMap<i64, Live>>>,
    /// The chats whose terminal interface is open, or that waited [`IDLE`], which have no Pi of their own meanwhile.
    /// Taken before `live` by whoever takes both, and held while a chat moves between the two.
    paused: Arc<Mutex<HashMap<i64, Paused>>>,
}

/// A run whose Pi has not ended yet.
#[derive(Clone)]
struct Live {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Child>>,
    /// What ends the wrapped process tree and cleans up after it, when the launcher wrapped this Pi.
    launched: Option<Arc<Launched>>,
    chat: bool,
    /// Why a chat's Pi was asked to exit, so that its exit is no failure.
    closing: Arc<Mutex<Option<Closing>>>,
    /// Since when a chat has waited for the person's next message; none while its agent works.
    waiting_since: Arc<Mutex<Option<Instant>>>,
    /// The agent as this Pi was started with it.
    agent: AgentConfig,
    run: RunningRun,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Closing {
    Stopped,
    Ended,
    /// The chat goes on in its terminal interface, or waits without a Pi: the exit ends nothing.
    Paused,
}

/// A chat without a Pi of its own: the run its Pi held, the follower of its session file while its terminal interface
/// is open (none for a chat that waited [`IDLE`]), whether the person ended or stopped the chat meanwhile, and the core,
/// to start its Pi again or end it.
struct Paused {
    run: RunningRun,
    follower: Option<Follower>,
    closing: Option<Closing>,
    app: Arc<App>,
}

impl Paused {
    /// Whether the chat waited [`IDLE`]: it has no terminal interface open and was not ended or stopped meanwhile.
    fn is_idle(&self) -> bool {
        self.follower.is_none() && self.closing.is_none()
    }
}

impl PiBolt {
    pub fn new(
        program: PathBuf,
        moebius: PathBuf,
        router_url: String,
        router: Arc<Router>,
        agents: BTreeMap<String, AgentConfig>,
    ) -> Self {
        Self {
            program,
            moebius,
            router_url,
            api_url: OnceLock::new(),
            router,
            agents: Arc::new(RwLock::new(AgentSet {
                agents,
                errors: BTreeMap::new(),
                skill_overrides: BTreeMap::new(),
            })),
            runners: Arc::new(Runners::default()),
            launcher: None,
            live: Arc::new(Mutex::new(HashMap::new())),
            paused: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Kills every Pi that is going, for a server that is about to restart; the runs they leave are failed by the next
    /// start, so that the board starts them again.
    pub fn kill_all(&self) {
        let all: Vec<Live> = lock(&self.live).values().cloned().collect();
        // Each wrapped run has its own stop and cleanup commands, which may take a while: they run side by side.
        std::thread::scope(|scope| {
            for live in &all {
                scope.spawn(move || {
                    kill_process(live);
                    finish_launch(live);
                    // Nothing runs `Drop` across the restart: close the tunnel and clear the runner's directory now.
                    if let Some(remote) = &live.run.remote {
                        remote.cleanup();
                    }
                });
            }
        });
    }

    /// The agents as they are now.
    fn agent_set(&self) -> std::sync::RwLockReadGuard<'_, AgentSet> {
        self.agents.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The chat `run_id`, if its Pi is going.
    fn live_chat(&self, run_id: i64) -> Option<Live> {
        lock(&self.live)
            .get(&run_id)
            .cloned()
            .filter(|live| live.chat)
    }
}

impl PiBolt {
    /// The agent `name` as a run uses it (see [`resolve_agent`]).
    fn agent(&self, app: &App, name: &str) -> Result<AgentConfig, StartError> {
        let agent = resolve_agent(&self.agent_set(), &app.data_dir, name)
            .map_err(StartError::UnknownAgent)?;
        Ok(tell_where_moebius_runs(app, name, agent))
    }

    /// The agent of the run `run_id`, a chat, as the chat runs it: with the model and reasoning level it was given in
    /// place of the agent's (see `ChatSettings`).
    fn chat_config(&self, app: &App, run_id: i64, agent: &str) -> Result<AgentConfig, String> {
        let settings = app.chat_settings(run_id).map_err(|e| e.to_string())?;
        chat_agent(&self.agent_set(), app, agent, &settings)
    }

    /// Starts Pi for what `launching` holds.
    fn launch_for(&self, app: &Arc<App>, launching: Launching) -> Result<(), String> {
        let Launching {
            config,
            run,
            card,
            prompt,
        } = launching;
        self.launch(app, card, config, run, prompt)
    }
}

impl CardRuns for PiBolt {
    fn start(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        purpose: (&str, Option<&str>),
        variant: Variant,
    ) -> Result<Run, StartError> {
        let config = self.agent(app, agent)?;
        let launch = |launching: Launching| self.launch_for(app, launching);
        let routing = (self.router.as_ref(), &self.runners);
        start_card(
            app,
            routing,
            card,
            (agent, config),
            purpose,
            variant,
            &launch,
        )
    }

    fn runners(&self) -> Vec<crate::harness::remote::RunnerInfo> {
        self.runners.list()
    }

    fn has_agent(&self, name: &str) -> bool {
        has_agent(&self.agent_set(), name)
    }

    fn agents(&self) -> Vec<AgentInfo> {
        agent_list(&self.agent_set())
    }

    fn token_budget(&self, agent: &str) -> Option<i64> {
        self.agent_set().agents.get(agent)?.token_budget
    }

    fn steer(&self, run_id: i64, message: &str) -> bool {
        let Some(live) = lock(&self.live).get(&run_id).cloned() else {
            return false;
        };
        send(
            &live.stdin,
            &json!({ "id": "moebius-steer", "type": "steer", "message": message }),
        )
    }

    /// Asks Pi to abort; the run ends with status `stopped` once Pi has settled. A Pi that does not exit within
    /// `STOP_GRACE` is killed.
    fn stop(&self, run_id: i64) -> bool {
        // Held throughout, so that the chat does not move to or from its terminal interface meanwhile.
        let mut paused = lock(&self.paused);
        if self.close_paused(&mut paused, run_id, Closing::Stopped) {
            return true;
        }
        let Some(live) = lock(&self.live).get(&run_id).cloned() else {
            return false;
        };
        send(
            &live.stdin,
            &json!({ "id": "moebius-abort", "type": "abort" }),
        );
        // A Pi that does not settle in time is killed, and its exit is no failure.
        *lock(&live.closing) = Some(Closing::Stopped);
        if live.chat {
            // A waiting chat has no turn to abort, so Pi is also asked to exit.
            lock(&live.stdin).take();
        }
        std::thread::spawn(move || {
            if !wait_for_exit(&live.child, STOP_GRACE) {
                kill_process(&live);
            }
        });
        true
    }
}

impl ImproveRuns for PiBolt {
    fn start_trial(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        definition: AgentConfig,
        proposal: i64,
        base: &str,
    ) -> Result<Run, StartError> {
        let launch = |launching: Launching| self.launch_for(app, launching);
        let routing = (self.router.as_ref(), &self.runners);
        let place = (proposal, base);
        start_trial(app, routing, card, (agent, &definition), place, &launch)
    }

    fn start_retro(
        &self,
        app: &Arc<App>,
        clone: &Path,
        base: &str,
        prompt: &str,
        files: &BTreeMap<String, String>,
    ) -> Result<Run, StartError> {
        let config = self.agent(app, RETRO_AGENT)?;
        let launch = |launching: Launching| self.launch_for(app, launching);
        let place = ("harness", clone, None);
        let purpose = (RETRO_AGENT, "retro");
        let start = (base, prompt, files);
        start_detached(app, &self.router, purpose, place, start, &config, &launch)
    }

    fn start_research(
        &self,
        app: &Arc<App>,
        repository: &Repository,
        base: &str,
        prompt: &str,
    ) -> Result<Run, StartError> {
        let config = self.agent(app, AUTORESEARCHER_AGENT)?;
        let launch = |launching: Launching| self.launch_for(app, launching);
        let clone = PathBuf::from(&repository.path);
        let place = (
            repository.name.as_str(),
            clone.as_path(),
            Some(repository.id),
        );
        let purpose = (AUTORESEARCHER_AGENT, "research");
        let start = (base, prompt, &BTreeMap::new());
        start_detached(app, &self.router, purpose, place, start, &config, &launch)
    }
}

impl ChatRuns for PiBolt {
    fn start_chat(
        &self,
        app: &Arc<App>,
        agent: &str,
        place: (Option<&Repository>, bool),
        message: Option<&str>,
        prompt: Option<&str>,
        origin: &str,
        settings: &ChatSettings,
    ) -> Result<Run, StartError> {
        self.open_chat(app, agent, place, (message, prompt), origin, settings)
    }

    fn change_chat(&self, app: &Arc<App>, run_id: i64) -> Result<(), String> {
        self.apply_chat_settings(app, run_id)
    }

    fn tui(&self, app: &Arc<App>, run_id: i64) -> Result<std::process::Command, String> {
        self.hand_to_tui(app, run_id)
    }

    fn close_tui(&self, app: &Arc<App>, run_id: i64) {
        self.take_back_from_tui(app, run_id)
    }

    fn say(&self, run_id: i64, message: &str) -> bool {
        self.tell_chat(run_id, message)
    }

    /// The chat's worktree and token are made again, and a Pi is started with what was said before, so that the chat
    /// carries on without being a new one.
    fn wake(&self, app: &Arc<App>, run_id: i64, prompt: &str) -> Result<(), String> {
        let stored = app.run(run_id).map_err(|e| e.to_string())?;
        let config = self.chat_config(app, run_id, &stored.agent)?;
        let running = woken_chat(app, &self.router, &stored, &config)?;
        launch_or_undo(app, &self.router, running, |run| {
            self.launch(app, None, &config, run, Some(prompt))
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    fn end(&self, run_id: i64) -> bool {
        self.end_chat(run_id)
    }
}

/// The directory of the run `run_id`, which holds its agent directory, sessions, bridge and the log of Pi's stderr.
fn run_dir(app: &App, run_id: i64) -> PathBuf {
    app.data_dir.join("runs").join(run_id.to_string())
}

/// Closes Pi's stdin, which asks it to shut down, waits up to [`STOP_GRACE`] for it to exit and kills it and what it
/// started.
fn shut_down(live: &Live) {
    lock(&live.stdin).take();
    wait_for_exit(&live.child, STOP_GRACE);
    // Also after a clean exit: what the agent started (a dev server, a watcher) must not outlive its run.
    kill_process(live);
    let _ = lock(&live.child).wait();
    finish_launch(live);
}

/// Runs the cleanup of a Pi the launcher wrapped, once, after the Pi has ended.
fn finish_launch(live: &Live) {
    if let Some(launched) = &live.launched {
        launched.finish();
    }
}

/// Kills Pi and everything it started (see `platform::own_group`); a wrapped Pi is ended by the launcher's stop
/// command first, which reaches the processes the wrapper's group does not.
fn kill_process(live: &Live) {
    if let Some(launched) = &live.launched {
        launched.stop();
    }
    let mut child = lock(&live.child);
    crate::platform::kill_tree(child.id());
    let _ = child.kill();
}

/// Writes one JSON line to Pi; false when its stdin is closed or broken.
fn send(stdin: &Mutex<Option<ChildStdin>>, value: &Value) -> bool {
    let mut guard = lock(stdin);
    let Some(pipe) = guard.as_mut() else {
        return false;
    };
    let mut line = value.to_string();
    line.push('\n');
    pipe.write_all(line.as_bytes())
        .and_then(|_| pipe.flush())
        .is_ok()
}

/// Waits up to `limit` for the process to exit; true when it did.
fn wait_for_exit(child: &Mutex<Child>, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        // Bound first, so the lock is not held while sleeping.
        let exited = lock(child).try_wait();
        match exited {
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => return false,
            _ => return true,
        }
    }
}

/// Locks a mutex, also when a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
