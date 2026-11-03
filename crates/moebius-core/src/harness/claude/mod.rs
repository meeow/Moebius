//! The Claude Code harness: each run is Claude Code's command line, `claude -p --input-format stream-json
//! --output-format stream-json --verbose`, in the run's worktree, with the router as its only provider
//! (`ANTHROPIC_BASE_URL`) and the run's token as its key, on the agent's model, with the agent's instructions
//! (`--append-system-prompt`), skills and MCP servers. Claude Code has no bridge, so the run's token is a checked one:
//! the router checks every tool call in the answers to it and replaces a refused one by a text that names the rule (see
//! `router`). Each message is a JSON line on its stdin; what it writes is mapped to the run events Pi-Bolt's runs have
//! (see `reader`), so the board, the desktop app and the Retro read both alike.
//!
//! A run of a card, of the Retro analyst or of the Autoresearcher ends after its answer: stdin is closed at the first
//! `result`, and the run ends when Claude Code exits; what the agent left is committed as for Pi-Bolt. A steer is
//! another user message, which Claude Code answers before it exits. A chat keeps its Claude Code between messages and
//! ends when the person ends or stops it; a chat given another model gets a new Claude Code that resumes the chat's
//! session (`--resume`). A chat on Claude Code has no terminal interface, does not pause when idle and ignores the
//! reasoning level, which Claude Code's command line does not take. Stopping a run kills its process.
//!
//! `process` writes a run's configuration directory, builds the command line and starts it; `reader` reads what it
//! writes and ends the run.

mod process;
mod reader;
// Unix only: the fake `claude` (testdata/fake-claude) is a Python script started by its `#!` line.
#[cfg(all(test, unix))]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::app::{App, ChatSettings};
use crate::board::{AgentInfo, CardRuns, ChatRuns, ImproveRuns, StartError, Variant};
use crate::config::{AgentConfig, AgentSet, HarnessKind};
use crate::harness::remote::Runners;
use crate::harness::run::{
    AUTORESEARCHER_AGENT, Launching, RETRO_AGENT, RunningRun, agent_list, chat_run, finish,
    has_agent, keep, launch_or_undo, resolve_agent, start_card, start_detached, start_trial,
    tell_where_moebius_runs, woken_chat,
};
use crate::router::Router;
use crate::store::{Card, Repository, Run};

/// Starts and stops Claude Code runs.
pub struct ClaudeCode {
    /// The `claude` executable.
    pub program: PathBuf,
    /// The `moebius` executable, which runs find as `$MOEBIUS`.
    pub moebius: PathBuf,
    /// The router's base URL, such as `http://127.0.0.1:7172`.
    pub router_url: String,
    /// The desktop API's base URL, which a run of the Moebius agent is told as `MOEBIUS_URL`; set once the server
    /// listens.
    pub api_url: OnceLock<String>,
    pub router: Arc<Router>,
    /// The agents, shared with Pi-Bolt.
    pub agents: Arc<RwLock<AgentSet>>,
    /// The machines an agent's `runner` can name, shared with Pi-Bolt.
    pub runners: Arc<Runners>,
    /// Whether a launcher is configured. Claude Code does not ask it yet, so with one no Claude Code run starts rather
    /// than one outside the launcher.
    pub launcher_configured: bool,
    live: Arc<Mutex<HashMap<i64, Live>>>,
}

/// A run whose Claude Code has not exited yet.
#[derive(Clone)]
struct Live {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Child>>,
    chat: bool,
    /// Why the process was asked to exit, so that its exit is no failure.
    closing: Arc<Mutex<Option<Closing>>>,
    /// The conversation's session, which a chat's next process resumes.
    session: String,
    /// Whether the session has had a message; Claude Code writes a session only then, so before it there is nothing
    /// to `--resume`.
    messaged: Arc<AtomicBool>,
    /// The agent as this process was started with it.
    agent: AgentConfig,
    run: RunningRun,
    app: Arc<App>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Closing {
    Stopped,
    Ended,
    /// A chat goes on in a new process: the exit ends nothing.
    Restarting,
}

impl ClaudeCode {
    pub fn new(
        program: PathBuf,
        moebius: PathBuf,
        router_url: String,
        router: Arc<Router>,
        agents: Arc<RwLock<AgentSet>>,
    ) -> Self {
        Self {
            program,
            moebius,
            router_url,
            api_url: OnceLock::new(),
            router,
            agents,
            runners: Arc::new(Runners::default()),
            launcher_configured: false,
            live: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Kills every Claude Code that is going, for a server that is about to restart.
    pub fn kill_all(&self) {
        let all: Vec<Live> = lock(&self.live).values().cloned().collect();
        for live in all {
            kill_process(&live);
            // Nothing runs `Drop` across the restart: close the tunnel and clear the runner's directory now.
            if let Some(remote) = &live.run.remote {
                remote.cleanup();
            }
        }
    }

    fn agent_set(&self) -> std::sync::RwLockReadGuard<'_, AgentSet> {
        self.agents.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The agent `name` as a run on Claude Code uses it, or why it cannot run on it.
    fn agent(&self, app: &App, name: &str) -> Result<AgentConfig, StartError> {
        let agent = resolve_agent(&self.agent_set(), &app.data_dir, name)
            .map_err(StartError::UnknownAgent)?;
        on_claude_code(tell_where_moebius_runs(app, name, agent), name)
    }

    /// Starts Claude Code for what `launching` holds, in a new session.
    fn launch_for(&self, app: &Arc<App>, launching: Launching) -> Result<(), String> {
        let Launching {
            config,
            run,
            card,
            prompt,
        } = launching;
        self.launch(
            app,
            card,
            config,
            run,
            (&process::new_session(), false),
            prompt,
        )
    }

    /// The chat `run_id`, if its Claude Code is going.
    fn live_chat(&self, run_id: i64) -> Option<Live> {
        lock(&self.live)
            .get(&run_id)
            .cloned()
            .filter(|live| live.chat)
    }
}

/// `agent` as Claude Code runs it, or why it cannot: Claude Code speaks only the anthropic protocol and loads no Pi
/// extensions.
fn on_claude_code(mut agent: AgentConfig, name: &str) -> Result<AgentConfig, StartError> {
    agent.harness = HarnessKind::ClaudeCode;
    match agent
        .harness
        .refuses(agent.protocol, !agent.extensions.is_empty())
    {
        Some(why) => Err(StartError::UnknownAgent(format!(
            "the agent {name} cannot run: {why}"
        ))),
        None => Ok(agent),
    }
}

impl CardRuns for ClaudeCode {
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

    /// Gives the agent the message as the next user message, which it answers before the run ends.
    fn steer(&self, run_id: i64, message: &str) -> bool {
        let Some(live) = lock(&self.live).get(&run_id).cloned() else {
            return false;
        };
        live.messaged.store(true, Ordering::SeqCst);
        send(&live.stdin, &user_message(&live.session, message))
    }

    /// Kills the run's Claude Code; the run ends as `stopped`.
    fn stop(&self, run_id: i64) -> bool {
        let Some(live) = lock(&self.live).get(&run_id).cloned() else {
            return false;
        };
        *lock(&live.closing) = Some(Closing::Stopped);
        lock(&live.stdin).take();
        kill_process(&live);
        true
    }
}

impl ImproveRuns for ClaudeCode {
    fn start_trial(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        definition: AgentConfig,
        proposal: i64,
        base: &str,
    ) -> Result<Run, StartError> {
        let config = on_claude_code(definition, agent)?;
        let launch = |launching: Launching| self.launch_for(app, launching);
        let routing = (self.router.as_ref(), &self.runners);
        let place = (proposal, base);
        start_trial(app, routing, card, (agent, &config), place, &launch)
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

impl ChatRuns for ClaudeCode {
    /// Claude Code has no interactive mode, so a chat of Claude Code has no terminal interface.
    fn tui(&self, _app: &Arc<App>, _run_id: i64) -> Result<std::process::Command, String> {
        Err("this harness has no terminal interface".into())
    }

    /// Nothing to take back: no terminal interface of Claude Code is open.
    fn close_tui(&self, _app: &Arc<App>, _run_id: i64) {}

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
        let config = on_claude_code(settings.apply(&self.agent(app, agent)?), agent)?;
        let running = chat_run(
            app,
            &self.router,
            (agent, &config),
            place,
            message,
            origin,
            settings,
        )?;
        launch_or_undo(app, &self.router, running, |run| {
            self.launch_for(
                app,
                Launching {
                    config: &config,
                    run,
                    card: None,
                    prompt,
                },
            )
        })
    }

    /// The chat's worktree and token are made again, and a Claude Code is started on a new session with what was said
    /// before, which is how every Claude Code chat starts.
    fn wake(&self, app: &Arc<App>, run_id: i64, prompt: &str) -> Result<(), String> {
        let stored = app.run(run_id).map_err(|e| e.to_string())?;
        let settings = app.chat_settings(run_id).map_err(|e| e.to_string())?;
        let config = on_claude_code(
            settings.apply(&self.agent(app, &stored.agent).map_err(|e| e.to_string())?),
            &stored.agent,
        )
        .map_err(|e| e.to_string())?;
        let running = woken_chat(app, &self.router, &stored, &config)?;
        launch_or_undo(app, &self.router, running, |run| {
            self.launch_for(
                app,
                Launching {
                    config: &config,
                    run,
                    card: None,
                    prompt: Some(prompt),
                },
            )
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// A chat given another model gets a new Claude Code on the same session; another reasoning level changes nothing.
    fn change_chat(&self, app: &Arc<App>, run_id: i64) -> Result<(), String> {
        let Some(live) = self.live_chat(run_id) else {
            return Err(format!("chat {run_id} has ended"));
        };
        let stored = app.run(run_id).map_err(|e| e.to_string())?;
        let settings = app.chat_settings(run_id).map_err(|e| e.to_string())?;
        let config = on_claude_code(
            settings.apply(&self.agent(app, &stored.agent).map_err(|e| e.to_string())?),
            &stored.agent,
        )
        .map_err(|e| e.to_string())?;
        if config.model == live.agent.model && config.protocol == live.agent.protocol {
            return Ok(());
        }
        {
            let mut closing = lock(&live.closing);
            if closing.is_some() {
                return Err(format!("chat {run_id} has ended"));
            }
            *closing = Some(Closing::Restarting);
        }
        lock(&live.stdin).take();
        kill_process(&live);
        let _ = lock(&live.child).wait();
        let resumed = self.launch(
            app,
            None,
            &config,
            live.run.clone(),
            (&live.session, live.messaged.load(Ordering::SeqCst)),
            None,
        );
        if let Err(e) = resumed {
            let why = format!("cannot start the chat's claude again with its new model: {e}");
            finish(app, live.run, &self.router, &self.live, Err(why.clone()));
            return Err(why);
        }
        Ok(())
    }

    fn say(&self, run_id: i64, message: &str) -> bool {
        let Some(live) = self.live_chat(run_id) else {
            return false;
        };
        tell(&live, message)
    }

    /// Ends a chat: its stdin is closed, so Claude Code exits once it has answered, and the chat ends then.
    fn end(&self, run_id: i64) -> bool {
        let Some(live) = self.live_chat(run_id) else {
            return false;
        };
        {
            let mut closing = lock(&live.closing);
            if closing.is_some() {
                return false;
            }
            *closing = Some(Closing::Ended);
        }
        lock(&live.stdin).take().is_some()
    }
}

/// Kills Claude Code and everything it started (see `platform::own_group`).
fn kill_process(live: &Live) {
    let mut child = lock(&live.child);
    crate::platform::kill_tree(child.id());
    let _ = child.kill();
}

/// Starts a thread that kills what Claude Code started once it has exited, also after a clean exit, so that a dev
/// server or watcher of its agent does not outlive its run. Such a process holds Claude Code's stdout open, so the
/// reader would not see the end of it. A chat that goes on in a new process is left alone.
fn kill_leftovers_after_exit(live: &Live) -> std::io::Result<()> {
    let live = live.clone();
    std::thread::Builder::new()
        .name(format!("claude leftovers {}", live.run.id))
        .spawn(move || {
            loop {
                // Bound first, so the lock is not held while sleeping.
                let exited = lock(&live.child).try_wait();
                match exited {
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    _ => break,
                }
            }
            if *lock(&live.closing) != Some(Closing::Restarting) {
                kill_process(&live);
            }
        })
        .map(drop)
}

/// Gives a chat's agent the person's message, after saying so with `agent_start`, so that the chat reads as working
/// until its answer.
fn tell(live: &Live, message: &str) -> bool {
    if lock(&live.stdin).is_none() {
        return false;
    }
    keep(&live.app, live.run.id, live.chat, "agent_start", &json!({}));
    live.messaged.store(true, Ordering::SeqCst);
    send(&live.stdin, &user_message(&live.session, message))
}

/// A user message as Claude Code's `stream-json` input takes it.
fn user_message(session: &str, text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        "parent_tool_use_id": null,
        "session_id": session,
    })
}

/// Writes one JSON line to Claude Code; false when its stdin is closed or broken.
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

/// Locks a mutex, also when a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
