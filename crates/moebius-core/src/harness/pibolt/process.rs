//! Starting Pi for a run: its agent directory (`models.json` with the router as the only provider, and `mcp.json` with
//! the MCP gateway, which serves the agent's MCP servers), the command line that gives it the agent's model,
//! instructions, skills, extensions and the bridge, and the process itself with the thread that reads it (see
//! `reader`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

use super::reader::Reader;
use super::{BRIDGE, LENS, Live, MOEBIUS_AGENT, PiBolt, kill_process, lock, run_dir, send};
use crate::app::{App, ChatImage};
use crate::bus::Event;
use crate::config::AgentConfig;
use crate::harness::launch::{Ask, Launched, Launcher, Unstarted, Wrapping};
use crate::harness::mcp;
use crate::harness::remote::Local;
use crate::harness::run::RunningRun;
use crate::router::Protocol;
use crate::store::Card;
use moebius_plugin_api::{LaunchRun, Urls, UrlsInside};

/// Where a Pi starts and what it is given: the worktree it works in, its agent directory and sessions, whether it
/// continues the latest session there, the lens and bridge extensions, the token of the router and whether it operates Moebius
/// with the desktop API.
pub(super) struct Place<'a> {
    pub(super) worktree: &'a Path,
    pub(super) agent_dir: &'a Path,
    pub(super) sessions: &'a Path,
    pub(super) continues: bool,
    pub(super) lens: &'a Path,
    pub(super) bridge: &'a Path,
    pub(super) token: &'a str,
    pub(super) operates: bool,
}

/// Where a run's Pi is started from, for the launcher: the agent's name, its card, its definition, the run's directory
/// and the file Pi's standard error goes to.
struct Spot<'a> {
    stored: &'a str,
    card: Option<&'a Card>,
    agent: &'a AgentConfig,
    run_dir: &'a Path,
    log: &'a Path,
}

impl PiBolt {
    /// The command that starts Pi for a run: `--mode rpc` for the run's own process, or, for the terminal interface of
    /// a chat, Pi's interactive mode. Both get the same provider, model, instructions, skills, MCP servers, extensions
    /// and bridge, so the checks see the same tool calls.
    pub(super) fn command(&self, agent: &AgentConfig, place: &Place, interactive: bool) -> Command {
        let mut command = Command::new(&self.program);
        // Not the server's whole environment: provider keys live in the router only.
        crate::harness::clear_environment(&mut command);
        command
            .current_dir(place.worktree)
            .env("PI_CODING_AGENT_DIR", place.agent_dir)
            .env("MOEBIUS", &self.moebius)
            .env("MOEBIUS_CHECK_URL", &self.router_url)
            .env("MOEBIUS_RUN_TOKEN", place.token)
            // How the bridge keeps the outputs of tools small (see `harness::output`).
            .env("MOEBIUS_TOOL_OUTPUT", json!(agent.tool_output).to_string())
            // What the lens checks after the agent's edits (see `harness::lens`).
            .env(
                "MOEBIUS_LENS",
                crate::harness::lens::plan_json(place.worktree),
            );
        if let (true, Some(url)) = (place.operates, self.api_url.get()) {
            // The Moebius agent operates Moebius with `$MOEBIUS`, as the run itself: its token is accepted by the
            // desktop API.
            command
                .env("MOEBIUS_URL", url)
                .env("MOEBIUS_TOKEN", place.token);
        }
        if !interactive {
            command.args(["--mode", "rpc"]);
        }
        command
            .args(["--offline", "--no-approve"])
            .args(["--provider", "moebius", "--model", &agent.model])
            .args(["--no-extensions", "--no-skills"])
            .arg("--session-dir")
            .arg(place.sessions);
        if place.continues {
            command.arg("--continue");
        }
        if let Some(thinking) = &agent.thinking {
            command.args(["--thinking", thinking]);
        }
        if let Some(instructions) = &agent.instructions {
            command.arg("--append-system-prompt").arg(instructions);
        }
        for skill in &agent.skills {
            command.arg("--skill").arg(skill);
        }
        // Pi's MCP support is an extension that `--no-extensions` turns off; it reads the `mcp.json` of the agent
        // directory, which holds the agent's servers.
        if !agent.mcp.is_empty() {
            command.args(["--extension", "builtin:mcp"]);
        }
        for extension in &agent.extensions {
            command.arg("--extension").arg(extension);
        }
        // The lens adds its findings to the results of edits; the bridge is last, so that it checks the input and
        // redacts the content the other extensions left, the lens's findings included.
        command.arg("--extension").arg(place.lens);
        command.arg("--extension").arg(place.bridge);
        command
    }

    /// Writes the agent directory `agent_dir`: `models.json` with the router as the `moebius` provider, and, if the agent
    /// has MCP servers, `mcp.json` with the MCP gateway, which is told the servers the run's token may use (without it
    /// Pi connects to none). Both hold the run's token, so only the server's user can read them. The model is marked as
    /// one that reasons when the agent has a reasoning level other than `off`, without which Pi would turn the level
    /// off, is given the level names of `[router.model_thinking]` when its endpoint takes any, and is marked as one
    /// that takes pictures when the router says its family does, without which Pi would drop a pasted picture.
    pub(super) fn write_agent_dir(
        &self,
        agent_dir: PathBuf,
        agent: &AgentConfig,
        token: &str,
    ) -> Result<PathBuf, String> {
        let gateway = format!("{}/mcp", self.router_url);
        self.write_agent_files(agent_dir, agent, token, (&self.router_url, &gateway))
    }

    /// Writes the agent directory as `write_agent_dir` does, with the router and the MCP gateway at `(router_url,
    /// gateway_url)`, which are the tunnel's end for a run on a runner and the launcher's addresses for a wrapped one.
    fn write_agent_files(
        &self,
        agent_dir: PathBuf,
        agent: &AgentConfig,
        token: &str,
        (router_url, gateway_url): (&str, &str),
    ) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&agent_dir).map_err(|e| e.to_string())?;
        if !agent.mcp.is_empty() {
            // The run reaches its servers only through the MCP gateway, which is given their values.
            let servers = mcp::with_values(&agent.mcp, &|name| std::env::var(name).ok())?;
            self.router.allow_mcp(token, servers);
            let gateway = mcp::pi_config(gateway_url, token);
            write_private(&agent_dir.join("mcp.json"), &gateway)?;
        }
        let (api, base_url) = match agent.protocol {
            Protocol::OpenAi => ("openai-completions", format!("{router_url}/v1")),
            Protocol::Anthropic => ("anthropic-messages", router_url.to_string()),
        };
        let mut model = json!({ "id": agent.model, "reasoning": reasons(agent) });
        // A model whose endpoint names the reasoning levels differently is given the value to send for each, which is
        // how it can be asked for a level Pi-Bolt would not send at all (`xhigh`). Without the map Pi sends the name
        // of the level, which the router passes to the upstream as it came.
        if let Some(levels) = self.router.thinking_map(&agent.model) {
            model["thinkingLevelMap"] = json!(levels);
        }
        // Pi sends the pictures of a message only for a model it is told takes them.
        if self.router.takes_images(&agent.model) {
            model["input"] = json!(["text", "image"]);
        }
        let models = json!({
            "providers": {
                "moebius": {
                    "baseUrl": base_url,
                    "api": api,
                    "apiKey": token,
                    "models": [model]
                }
            }
        });
        write_private(&agent_dir.join("models.json"), &models)?;
        Ok(agent_dir)
    }

    /// Starts Pi for a run whose worktree and token exist, and the thread that reads it. Pi is given the `prompt` to
    /// answer and the `images` sent with it; without a prompt, a chat's Pi continues the chat's session and the chat
    /// waits for the person's next message.
    pub(super) fn launch(
        &self,
        app: &Arc<App>,
        card: Option<&Card>,
        agent: &AgentConfig,
        run: RunningRun,
        (prompt, images): (Option<&str>, &[ChatImage]),
    ) -> Result<(), String> {
        app.store
            .set_run_worktree(run.id, &run.worktree.to_string_lossy())
            .map_err(|e| e.to_string())?;
        let run_dir = run_dir(app, run.id);
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
        let gateway_url = format!("{router_url}/mcp");
        let agent_dir = self
            .write_agent_files(
                run_dir.join("agent"),
                agent,
                &run.token,
                (router_url, &gateway_url),
            )
            .map_err(|e| format!("cannot write the agent directory: {e}"))?;
        let bridge = run_dir.join("moebius-bridge.ts");
        std::fs::write(&bridge, BRIDGE)
            .map_err(|e| format!("cannot write the bridge extension: {e}"))?;
        let lens = run_dir.join("moebius-lens.ts");
        std::fs::write(&lens, LENS).map_err(|e| format!("cannot write the lens extension: {e}"))?;
        let log = run_dir.join("stderr.log");
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| e.to_string())?;
        let stored = app.store.run(run.id).map_err(|e| e.to_string())?;
        let operates = stored.as_ref().is_some_and(|s| s.agent == MOEBIUS_AGENT);
        let sessions = run_dir.join("sessions");
        let place = Place {
            worktree: &run.worktree,
            agent_dir: &agent_dir,
            sessions: &sessions,
            continues: prompt.is_none(),
            lens: &lens,
            bridge: &bridge,
            token: &run.token,
            operates,
        };
        let mut command = self.command(agent, &place, false);
        let mut launched = None;
        if let Some(launcher) = &self.launcher {
            let stored = stored.as_ref().ok_or("the run is gone")?;
            let spot = Spot {
                stored: &stored.agent,
                card,
                agent,
                run_dir: &run_dir,
                log: &log,
            };
            launched = self.through_launcher(app, launcher, &run, &place, spot, &mut command)?;
            if let Some((urls, _)) = &launched {
                if urls.router.is_some() || urls.gateway.is_some() {
                    let inside_router = urls.router.as_deref().unwrap_or(router_url);
                    let inside_gateway =
                        urls.gateway.clone().unwrap_or_else(|| match &urls.router {
                            Some(router) => format!("{router}/mcp"),
                            None => gateway_url.clone(),
                        });
                    self.write_agent_files(
                        agent_dir.clone(),
                        agent,
                        &run.token,
                        (inside_router, &inside_gateway),
                    )
                    .map_err(|e| format!("cannot write the agent directory: {e}"))?;
                }
                if let Some(check) = &urls.check {
                    command.env("MOEBIUS_CHECK_URL", check);
                }
            }
        }
        let launched = launched.map(|(_, launched)| launched);
        // Whatever fails before Pi has started cleans up after the launcher.
        let unstarted = Unstarted::new(launched.as_ref());
        if let Some(remote) = &remote {
            let local = Local {
                worktree: &run.worktree,
                run_dir: &run_dir,
                router_url: &self.router_url,
            };
            command = remote.command(&command, &local)?;
        }
        let program = match &launched {
            Some(_) => command.get_program().to_string_lossy().into_owned(),
            None => self.program.display().to_string(),
        };
        let mut child = crate::platform::own_group(&mut command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|e| format!("cannot start {program}: {e}"))?;
        unstarted.started();
        let stdout = child.stdout.take().expect("stdout is piped");
        let live = Live {
            launched,
            stdin: Arc::new(Mutex::new(child.stdin.take())),
            child: Arc::new(Mutex::new(child)),
            chat: run.purpose == "chat",
            closing: Arc::new(Mutex::new(None)),
            waiting_since: Arc::new(Mutex::new(prompt.is_none().then(Instant::now))),
            agent: agent.clone(),
            run: run.clone(),
        };
        lock(&self.live).insert(run.id, live.clone());
        let begun = self.begin(app, card, &live, (prompt, images), stdout);
        if begun.is_err() {
            // Nothing reads this Pi: end it and what it started, so that it is not orphaned.
            lock(&self.live).remove(&run.id);
            lock(&live.stdin).take();
            kill_process(&live);
            let _ = lock(&live.child).wait();
            super::finish_launch(&live);
        }
        begun
    }

    /// Asks the launcher how to start Pi for `run`, which `command` starts as it is. When the launcher wraps it,
    /// `command` becomes the wrapped one, and the addresses Pi sees and what ends it are returned. An `Err` fails the
    /// run: with a launcher configured, Pi is never started without its answer.
    fn through_launcher(
        &self,
        app: &App,
        launcher: &Launcher,
        run: &RunningRun,
        place: &Place,
        spot: Spot,
        command: &mut Command,
    ) -> Result<Option<(UrlsInside, Arc<Launched>)>, String> {
        let remote = run.remote.as_ref().map(|remote| remote.runner());
        let mut read_only = vec![
            place.agent_dir.to_path_buf(),
            place.lens.to_path_buf(),
            place.bridge.to_path_buf(),
        ];
        read_only.extend(spot.agent.extensions.iter().cloned());
        read_only.extend(spot.agent.skills.iter().cloned());
        let mut read_write = vec![place.worktree.to_path_buf(), place.sessions.to_path_buf()];
        if remote.is_none() {
            // Pi's temporary files go to a directory of the run, which only it can write, not to the shared one.
            let tmp = spot.run_dir.join("tmp");
            std::fs::create_dir_all(&tmp)
                .map_err(|e| format!("cannot make the temporary directory of the run: {e}"))?;
            for name in ["TMPDIR", "TEMP", "TMP"] {
                command.env(name, &tmp);
            }
            read_write.push(tmp);
        }
        let ask = Ask {
            run: LaunchRun {
                id: run.id,
                purpose: run.purpose.clone(),
                agent: spot.stored.to_string(),
                chat: run.purpose == "chat",
                card: spot.card.map(|card| card.id),
            },
            dir: place.worktree,
            read_only,
            read_write,
            urls: Urls {
                router: self.router_url.clone(),
                gateway: format!("{}/mcp", self.router_url),
                check: self.router_url.clone(),
            },
            clone: run.clone.as_deref(),
            runner: remote,
            log: spot.log,
        };
        let (note, wrapping) = launcher.start(ask, command)?;
        if let Some(note) = note {
            let _ = app.record_run_event(run.id, "launcher_note", &json!({ "note": note }));
        }
        Ok(wrapping.map(
            |Wrapping {
                 command: wrapped,
                 launched,
                 urls,
             }| {
                *command = wrapped;
                (urls, launched)
            },
        ))
    }

    /// Marks the run as started, gives Pi its prompt and starts the thread that reads it.
    fn begin(
        &self,
        app: &Arc<App>,
        card: Option<&Card>,
        live: &Live,
        (prompt, images): (Option<&str>, &[ChatImage]),
        stdout: impl Read + Send + 'static,
    ) -> Result<(), String> {
        let run = live.run.clone();
        match prompt {
            Some(prompt) => {
                app.store
                    .set_run_status(run.id, "running")
                    .map_err(|e| e.to_string())?;
                app.bus.publish(Event::RunStarted {
                    run: run.id,
                    card: card.map(|card| card.id),
                });
                let mut said =
                    json!({ "id": "moebius-prompt", "type": "prompt", "message": prompt });
                if !images.is_empty() {
                    said["images"] = json!(
                        images
                            .iter()
                            .map(|image| json!({
                                "type": "image",
                                "data": image.base64(),
                                "mimeType": image.media_type,
                            }))
                            .collect::<Vec<_>>()
                    );
                }
                send(&live.stdin, &said);
            }
            None => app
                .store
                .set_run_status(run.id, "waiting")
                .map_err(|e| e.to_string())?,
        }
        let reader = Reader {
            app: app.clone(),
            run,
            live: live.clone(),
            router: self.router.clone(),
            lives: self.live.clone(),
        };
        let name = format!("pi-bolt run {}", reader.run.id);
        std::thread::Builder::new()
            .name(name)
            .spawn(move || reader.read(stdout))
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// Whether `agent` reasons: it has a reasoning level other than `off`.
pub(super) fn reasons(agent: &AgentConfig) -> bool {
    agent
        .thinking
        .as_deref()
        .is_some_and(|level| level != "off")
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

// Unix only: the fake Pi is a Python script started by its `#!` line.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::board::{CardRuns, Variant};
    use crate::harness::mcp::McpServer;
    use crate::harness::pibolt::testing::{child_pid, setup, wait_until_ended, wait_until_gone};
    use crate::repos::git;
    use std::collections::BTreeMap;

    // Linux only: whether the fake's `sleep` runs is read from `/proc`.
    #[cfg(target_os = "linux")]
    #[test]
    fn stopping_a_run_that_does_not_settle_ends_it_stopped_and_kills_what_it_started() {
        let (_dir, app, pi, repo) = setup();
        // The fake never answers `abort`, as a Pi stuck in a tool does not.
        let card = app
            .create_issue(repo, "Please [deaf]", "", "person")
            .unwrap();
        let run = pi
            .start(&app, &card, "Coder", ("code", None), Variant::default())
            .unwrap();
        let child = child_pid(&app, run.id);
        assert!(crate::platform::is_running(child));
        assert!(pi.stop(run.id));
        // Killed after STOP_GRACE: stopped, not failed, which the board would start again.
        let run = wait_until_ended(&app, run.id);
        assert_eq!(run.status, "stopped", "{run:?}");
        wait_until_gone("the background process is killed", child);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_stopped_run_leaves_no_background_process_of_its_agent() {
        let (_dir, app, pi, repo) = setup();
        let card = app
            .create_issue(repo, "Please [child]", "", "person")
            .unwrap();
        let run = pi
            .start(&app, &card, "Coder", ("code", None), Variant::default())
            .unwrap();
        let child = child_pid(&app, run.id);
        assert!(pi.stop(run.id));
        // Pi settles and exits at once; what it started goes with it.
        let run = wait_until_ended(&app, run.id);
        assert_eq!(run.status, "stopped");
        wait_until_gone("the background process is killed", child);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_launch_that_fails_after_pi_started_kills_it() {
        let (_dir, app, pi, _repo) = setup();
        // The chat cannot be marked as waiting once Pi runs.
        app.store.execute_batch_for_test(
            "CREATE TRIGGER no_waiting BEFORE UPDATE OF status ON runs WHEN NEW.status = 'waiting' \
             BEGIN SELECT RAISE(ABORT, 'refused'); END",
        );
        let started = crate::board::ChatRuns::start_chat(
            &pi,
            &app,
            "Coder",
            (None, false),
            None,
            &[],
            None,
            "chat",
            &crate::app::ChatSettings::default(),
        );
        assert!(started.is_err());
        assert!(lock(&pi.live).is_empty(), "the Pi stays in live");
        // No process of the run's agent directory is left.
        let marker = app.data_dir.join("runs").to_string_lossy().into_owned();
        assert!(
            crate::harness::pibolt::testing::processes_with_env(&marker).is_empty(),
            "an orphaned Pi runs"
        );
    }

    #[test]
    fn a_run_is_stored_event_by_event_and_its_work_committed() {
        let (_dir, app, pi, repo) = setup();
        let card = app
            .create_issue(repo, "Write done.txt", "", "person")
            .unwrap();
        let run = pi
            .start(&app, &card, "Coder", ("code", None), Variant::default())
            .unwrap();
        assert_eq!(run.status, "running");
        let run = wait_until_ended(&app, run.id);
        assert_eq!(run.status, "succeeded");
        // The summary is the text of the last assistant message that agent_end reports.
        assert_eq!(run.summary.as_deref(), Some("line one line two"));

        let events = app.store.events(run.id).unwrap();
        let kinds: Vec<_> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "response",
                "agent_start",
                "message_update",
                "tool_execution_start",
                "tool_execution_end",
                "agent_end",
                "agent_settled"
            ]
        );
        // U+2028 inside a string is part of the record, not a line break.
        assert_eq!(
            events[2].data["assistantMessageEvent"]["delta"],
            "line one\u{2028}line two"
        );

        let clone = PathBuf::from(&app.store.repository(repo).unwrap().unwrap().path);
        let branch = format!("moebius/run-{}", run.id);
        assert_eq!(
            git(&clone, &["show", &format!("{branch}:done.txt")]).unwrap(),
            "done"
        );
        assert!(!PathBuf::from(run.worktree.unwrap()).exists());

        let run_dir = app.data_dir.join("runs").join(run.id.to_string());
        let argv: Vec<String> =
            serde_json::from_slice(&std::fs::read(run_dir.join("agent/argv.json")).unwrap())
                .unwrap();
        assert!(argv.windows(2).any(|w| w == ["--model", "qwen-local"]));
        assert!(argv.iter().any(|a| a == "--no-approve") && argv.iter().any(|a| a == "--offline"));
        assert!(argv.windows(2).any(|w| w == ["--thinking", "low"]));
        // Only the configured extensions and skills load, and the bridge is the last extension.
        let bridge = run_dir.join("moebius-bridge.ts");
        assert!(
            argv.iter().any(|a| a == "--no-extensions") && argv.iter().any(|a| a == "--no-skills")
        );
        let extensions: Vec<_> = argv.windows(2).filter(|w| w[0] == "--extension").collect();
        assert_eq!(extensions[0][1], "/agent/own.ts");
        assert_eq!(extensions.last().unwrap()[1], bridge.to_str().unwrap());
        assert_eq!(std::fs::read_to_string(&bridge).unwrap(), BRIDGE);
        // The lens comes just before the bridge, and is told what to check.
        let lens = run_dir.join("moebius-lens.ts");
        assert_eq!(extensions[extensions.len() - 2][1], lens.to_str().unwrap());
        assert_eq!(std::fs::read_to_string(&lens).unwrap(), LENS);
        let env: Value =
            serde_json::from_slice(&std::fs::read(run_dir.join("agent/env.json")).unwrap())
                .unwrap();
        assert_eq!(env["MOEBIUS"], "/usr/local/bin/moebius");
        // The server's environment does not reach Pi (cargo sets this variable for the test), but its PATH does.
        assert!(env["CARGO_MANIFEST_DIR"].is_null(), "{env}");
        assert!(env["PATH"].is_string());
        assert_eq!(env["MOEBIUS_CHECK_URL"], "http://127.0.0.1:9");
        let plan: Value = serde_json::from_str(env["MOEBIUS_LENS"].as_str().unwrap()).unwrap();
        assert_eq!(plan["enabled"], true);
        assert_eq!(plan["budget_ms"], 1500);
        let tool_output: Value =
            serde_json::from_str(env["MOEBIUS_TOOL_OUTPUT"].as_str().unwrap()).unwrap();
        assert_eq!(
            tool_output,
            json!({"hide_after_turns": 10, "max_lines": 200, "max_bytes": 16384, "compress": true})
        );
        assert!(
            env["MOEBIUS_RUN_TOKEN"]
                .as_str()
                .unwrap()
                .starts_with("moebius-run-")
        );
        assert!(
            argv.windows(2)
                .any(|w| w == ["--append-system-prompt", "Be brief."])
        );
        let models: Value =
            serde_json::from_slice(&std::fs::read(run_dir.join("agent/models.json")).unwrap())
                .unwrap();
        // The run's token is in the file, so only the server's user can read it.
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(run_dir.join("agent/models.json"))
                .unwrap()
                .permissions();
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
                0o600
            );
        }
        let provider = &models["providers"]["moebius"];
        assert_eq!(provider["baseUrl"], "http://127.0.0.1:9/v1");
        assert!(
            provider["apiKey"]
                .as_str()
                .unwrap()
                .starts_with("moebius-run-")
        );
    }

    #[test]
    fn a_run_gets_exactly_the_skills_and_mcp_servers_of_its_agent() {
        let (_dir, app, pi, repo) = setup();
        let server = |text: &str| {
            let servers = mcp::parse_file(&format!("[s]\n{text}"), &["CARGO_PKG_NAME".to_string()]);
            servers.unwrap().remove("s").unwrap().unwrap()
        };
        let agent = |skill: &str, mcp: &[(&str, McpServer)]| AgentConfig {
            skills: vec![format!("/skills/{skill}").into()],
            extensions: vec![],
            mcp: mcp
                .iter()
                .map(|(name, server)| (name.to_string(), server.clone()))
                .collect(),
            ..pi.agent_set().agents["Coder"].clone()
        };
        let docs = server("command = \"docs\"\nargs = [\"--name\", \"${CARGO_PKG_NAME}\"]\n");
        let search = server(
            "url = \"https://search.test/mcp\"\nheaders = { Name = \"${CARGO_PKG_NAME}\" }\n",
        );
        let agents = [
            ("A".to_string(), agent("a", &[("docs", docs)])),
            ("B".to_string(), agent("b", &[("search", search)])),
            ("C".to_string(), agent("c", &[])),
        ];
        pi.agents.write().unwrap().agents.extend(agents);
        let run_agent = |name: &str| {
            let card = app
                .create_issue(repo, "Write done.txt", "", "person")
                .unwrap();
            let run = pi
                .start(&app, &card, name, ("code", None), Variant::default())
                .unwrap();
            let run = wait_until_ended(&app, run.id);
            let agent_dir = app
                .data_dir
                .join("runs")
                .join(run.id.to_string())
                .join("agent");
            let argv: Vec<String> =
                serde_json::from_slice(&std::fs::read(agent_dir.join("argv.json")).unwrap())
                    .unwrap();
            (argv, agent_dir.join("mcp.json"))
        };
        let values = |argv: &[String], flag: &str| -> Vec<String> {
            argv.windows(2)
                .filter(|w| w[0] == flag)
                .map(|w| w[1].clone())
                .collect()
        };

        let (argv, mcp_json) = run_agent("A");
        assert_eq!(values(&argv, "--skill"), ["/skills/a"]);
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&mcp_json).unwrap().permissions();
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
                0o600
            );
        }
        // The run is given only the gateway, with its token; the servers and their values stay with the server.
        let text = std::fs::read_to_string(mcp_json).unwrap();
        assert!(
            !text.contains("moebius-core") && !text.contains("docs"),
            "{text}"
        );
        let config: Value = serde_json::from_str(&text).unwrap();
        let gateway = &config["mcpServers"]["moebius"];
        assert_eq!(config["mcpServers"].as_object().unwrap().len(), 1);
        assert_eq!(gateway["url"], "http://127.0.0.1:9/mcp");
        assert_eq!(gateway["exposure"], "direct");
        assert!(
            gateway["headers"]["Authorization"]
                .as_str()
                .unwrap()
                .starts_with("Bearer moebius-run-")
        );
        // Pi's MCP support is switched on explicitly, before the bridge, which is still last.
        let extensions = values(&argv, "--extension");
        assert_eq!(extensions[0], "builtin:mcp");
        assert!(extensions.last().unwrap().ends_with("moebius-bridge.ts"));

        let (argv, mcp_json) = run_agent("B");
        assert_eq!(values(&argv, "--skill"), ["/skills/b"]);
        let text = std::fs::read_to_string(mcp_json).unwrap();
        assert!(!text.contains("search.test"), "{text}");

        // An agent without servers gets no mcp.json and no MCP support.
        let (argv, mcp_json) = run_agent("C");
        assert_eq!(values(&argv, "--skill"), ["/skills/c"]);
        assert!(!mcp_json.exists());
        // Only the lens and the bridge.
        let extensions = values(&argv, "--extension");
        assert_eq!(extensions.len(), 2);
        assert!(extensions[0].ends_with("moebius-lens.ts"));
    }

    #[test]
    fn the_gateway_is_given_the_servers_of_the_runs_token_with_their_values() {
        let (dir, _app, pi, _repo) = setup();
        let servers = mcp::parse_file(
            "[docs]\ncommand = \"docs\"\nargs = [\"${CARGO_PKG_NAME}\"]\n",
            &["CARGO_PKG_NAME".to_string()],
        )
        .unwrap();
        let agent = AgentConfig {
            mcp: servers
                .into_iter()
                .map(|(name, server)| (name, server.unwrap()))
                .collect(),
            ..pi.agent_set().agents["Coder"].clone()
        };
        let token = pi.router.issue_token(crate::router::Grant {
            run_id: 1,
            card_id: None,
            budget: None,
        });
        pi.write_agent_dir(dir.path().join("agent"), &agent, &token)
            .unwrap();
        let (_, given) = pi.router.mcp_grant(&token).unwrap();
        assert_eq!(given["docs"].args, ["moebius-core"]);
        pi.router.revoke_token(&token);
        assert!(pi.router.mcp_grant(&token).is_none());
    }

    #[test]
    fn the_other_level_names_of_a_model_are_written_into_its_models_json() {
        let (dir, _app, pi, _repo) = setup();
        let token = pi.router.issue_token(crate::router::Grant {
            run_id: 1,
            card_id: None,
            budget: None,
        });
        let agent = AgentConfig {
            model: "qwen-local".into(),
            thinking: Some("xhigh".into()),
            ..pi.agent_set().agents["Coder"].clone()
        };
        let written = |dir: &Path, agent: &AgentConfig| {
            pi.write_agent_dir(dir.to_path_buf(), agent, &token)
                .unwrap();
            let file = dir.join("models.json");
            serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap()
        };
        let model = |dir: &Path, agent: &AgentConfig| {
            written(dir, agent)["providers"]["moebius"]["models"][0].clone()
        };
        // Nothing is told about the model, so Pi is given no map and sends the name of the level itself.
        assert_eq!(
            model(&dir.path().join("agent"), &agent),
            json!({"id": "qwen-local", "reasoning": true})
        );

        pi.router.set_model_thinking(BTreeMap::from([(
            "qwen-local".to_string(),
            BTreeMap::from([
                ("high".to_string(), "xhigh".to_string()),
                ("xhigh".to_string(), "xhigh".to_string()),
            ]),
        )]));
        assert_eq!(
            model(&dir.path().join("agent"), &agent)["thinkingLevelMap"],
            json!({"high": "xhigh", "xhigh": "xhigh"})
        );
        // A model nobody spoke of keeps the file it had, so an endpoint that takes the levels' names is unaffected.
        let other = AgentConfig {
            model: "another-model".into(),
            ..agent.clone()
        };
        assert_eq!(
            model(&dir.path().join("other"), &other),
            json!({"id": "another-model", "reasoning": true})
        );
    }

    /// Pi drops the pictures of a message for a model it is not told takes them, so the agent directory has to say
    /// which of its models looks at them.
    #[test]
    fn the_agent_dir_marks_which_of_its_models_looks_at_pictures() {
        let (dir, _app, pi, _repo) = setup();
        for (model, looks) in [("qwen-local", false), ("claude-sonnet-4-5", true)] {
            let agent = AgentConfig {
                model: model.into(),
                ..pi.agent_set().agents["Coder"].clone()
            };
            let agent_dir = pi
                .write_agent_dir(dir.path().join(model), &agent, "token")
                .unwrap();
            let models: Value =
                serde_json::from_slice(&std::fs::read(agent_dir.join("models.json")).unwrap())
                    .unwrap();
            let marked = models["providers"]["moebius"]["models"][0].get("input")
                == Some(&json!(["text", "image"]));
            assert_eq!(
                marked, looks,
                "{model} is told the wrong thing about pictures"
            );
        }
    }
}
