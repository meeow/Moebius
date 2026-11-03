//! Starting Claude Code for a run: its configuration directory (`CLAUDE_CONFIG_DIR`, the run's own, with the agent's
//! skills linked into `skills/` and its sessions), `mcp.json` with the agent's MCP servers, the command line that gives
//! it the agent's model and instructions, the router and the run's token, and the process itself with the thread that
//! reads it (see `reader`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::reader::Reader;
use super::{ClaudeCode, Live, lock, send, user_message};
use crate::app::{App, ChatImage};
use crate::bus::Event;
use crate::config::AgentConfig;
use crate::harness::mcp;
use crate::harness::remote::Local;
use crate::harness::run::{MOEBIUS_AGENT, RunningRun, keep};
use crate::store::Card;

/// A new session id for Claude Code's `--session-id`: a random UUID.
pub(super) fn new_session() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Where a Claude Code starts and what it is given.
pub(super) struct Place<'a> {
    pub(super) worktree: &'a Path,
    pub(super) config_dir: &'a Path,
    /// `mcp.json`, when the agent has MCP servers.
    pub(super) mcp: Option<&'a Path>,
    /// The session it starts, or resumes when `resumes` is set.
    pub(super) session: &'a str,
    pub(super) resumes: bool,
    pub(super) token: &'a str,
    /// Whether it operates Moebius with the desktop API, as the Moebius agent does.
    pub(super) operates: bool,
}

impl ClaudeCode {
    /// The command that starts Claude Code for a run. It reads its settings only from its own configuration directory
    /// and connects only to the MCP servers it is given, and never asks before a tool call: the router checks them.
    pub(super) fn command(&self, agent: &AgentConfig, place: &Place) -> Command {
        let mut command = Command::new(&self.program);
        command
            .current_dir(place.worktree)
            .env("CLAUDE_CONFIG_DIR", place.config_dir)
            .env("ANTHROPIC_BASE_URL", &self.router_url)
            .env("ANTHROPIC_API_KEY", place.token)
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .env_remove("CLAUDE_CODE_USE_BEDROCK")
            .env_remove("CLAUDE_CODE_USE_VERTEX")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("MOEBIUS", &self.moebius)
            .env("MOEBIUS_RUN_TOKEN", place.token);
        if let (true, Some(url)) = (place.operates, self.api_url.get()) {
            command
                .env("MOEBIUS_URL", url)
                .env("MOEBIUS_TOKEN", place.token);
        }
        command
            .args(["-p", "--input-format", "stream-json"])
            .args(["--output-format", "stream-json", "--verbose"])
            .args(["--model", &agent.model])
            .args(["--permission-mode", "bypassPermissions"])
            .args(["--setting-sources", "user", "--strict-mcp-config"])
            .arg(if place.resumes {
                "--resume"
            } else {
                "--session-id"
            })
            .arg(place.session);
        if let Some(instructions) = &agent.instructions {
            command.arg("--append-system-prompt").arg(instructions);
        }
        if let Some(mcp) = place.mcp {
            command.arg("--mcp-config").arg(mcp);
        }
        command
    }

    /// Writes the run's configuration directory `config_dir`, with each of the agent's skills linked (copied on Windows)
    /// into `skills/` under its directory's name, and `mcp_file` with the MCP gateway and the run's `token`, if the agent
    /// has MCP servers; returns the file then. The gateway is told the servers with their values; only the server's user can read the file.
    /// The router is at `router_url`, which is the tunnel's end for a run on a runner.
    pub(super) fn write_config(
        &self,
        config_dir: &Path,
        mcp_file: PathBuf,
        agent: &AgentConfig,
        (token, router_url): (&str, &str),
    ) -> Result<Option<PathBuf>, String> {
        let skills = config_dir.join("skills");
        std::fs::create_dir_all(&skills).map_err(|e| e.to_string())?;
        for skill in &agent.skills {
            let Some(name) = skill.file_name() else {
                continue;
            };
            let link = skills.join(name);
            // A link from an earlier start of the run stays, even if what it leads to is not there.
            if link.symlink_metadata().is_err() {
                crate::platform::link_dir(skill, &link).map_err(|e| e.to_string())?;
            }
        }
        if agent.mcp.is_empty() {
            return Ok(None);
        }
        // The run reaches its servers only through the MCP gateway, which is given their values.
        let servers = mcp::with_values(&agent.mcp, &|name| std::env::var(name).ok())?;
        self.router.allow_mcp(token, servers);
        let gateway = mcp::claude_config(&format!("{router_url}/mcp"), token);
        write_private(&mcp_file, &gateway)?;
        Ok(Some(mcp_file))
    }

    /// Starts Claude Code for a run whose worktree and token exist, on the session `session` (resumed when the flag
    /// is set), and the thread that reads it. It is given the `prompt` to answer and the `images` sent with it;
    /// without a prompt, a chat waits for the person's message.
    pub(super) fn launch(
        &self,
        app: &Arc<App>,
        card: Option<&Card>,
        agent: &AgentConfig,
        run: RunningRun,
        (session, resumes): (&str, bool),
        (prompt, images): (Option<&str>, &[ChatImage]),
    ) -> Result<(), String> {
        app.store
            .set_run_worktree(run.id, &run.worktree.to_string_lossy())
            .map_err(|e| e.to_string())?;
        let run_dir = app.data_dir.join("runs").join(run.id.to_string());
        let config_dir = run_dir.join("claude");
        // On a runner the run reaches the router through the tunnel, whose end is where the files say it is.
        let remote = run.remote.clone();
        if let (Some(remote), Some(card)) = (&remote, card) {
            remote.connect(&self.router_url)?;
            remote.prepare(&run.worktree, &format!("repo-{}", card.repository_id))?;
            app.store
                .set_run_runner(run.id, remote.runner())
                .map_err(|e| e.to_string())?;
        }
        let router_url = remote
            .as_ref()
            .and_then(|remote| remote.router_url())
            .unwrap_or(&self.router_url);
        let mcp = self
            .write_config(
                &config_dir,
                run_dir.join("mcp.json"),
                agent,
                (&run.token, router_url),
            )
            .map_err(|e| format!("cannot write the configuration directory: {e}"))?;
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(run_dir.join("stderr.log"))
            .map_err(|e| e.to_string())?;
        let operates = app
            .store
            .run(run.id)
            .map_err(|e| e.to_string())?
            .is_some_and(|stored| stored.agent == MOEBIUS_AGENT);
        let place = Place {
            worktree: &run.worktree,
            config_dir: &config_dir,
            mcp: mcp.as_deref(),
            session,
            resumes,
            token: &run.token,
            operates,
        };
        let mut command = self.command(agent, &place);
        if let Some(remote) = &remote {
            let local = Local {
                worktree: &run.worktree,
                run_dir: &run_dir,
                router_url: &self.router_url,
            };
            command = remote.command(&command, &local)?;
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", self.program.display()))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let chat = run.purpose == "chat";
        let live = Live {
            stdin: Arc::new(Mutex::new(child.stdin.take())),
            child: Arc::new(Mutex::new(child)),
            chat,
            closing: Arc::new(Mutex::new(None)),
            session: session.to_string(),
            agent: agent.clone(),
            run: run.clone(),
            app: app.clone(),
        };
        lock(&self.live).insert(run.id, live.clone());
        match prompt {
            Some(prompt) => {
                app.store
                    .set_run_status(run.id, "running")
                    .map_err(|e| e.to_string())?;
                app.bus.publish(Event::RunStarted {
                    run: run.id,
                    card: card.map(|card| card.id),
                });
                keep(app, run.id, chat, "agent_start", &json!({}));
                send(&live.stdin, &user_message(session, prompt, images));
            }
            None => app
                .store
                .set_run_status(run.id, "waiting")
                .map_err(|e| e.to_string())?,
        }
        let reader = Reader {
            app: app.clone(),
            live,
            router: self.router.clone(),
            lives: self.live.clone(),
        };
        let name = format!("claude run {}", run.id);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || reader.read(stdout))
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// Writes `value` as JSON to a new `file` that only the server's user can read.
fn write_private(file: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    crate::platform::owner_only(
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )
    .open(file)
    .and_then(|mut file| file.write_all(&text))
    .map_err(|e| e.to_string())
}
