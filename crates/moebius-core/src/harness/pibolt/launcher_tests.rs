//! A launcher wrapping Pi: "unchanged" starts it as without one, a wrapped launch is started with its stop and
//! cleanup commands and the addresses it names, a launcher that fails, hangs or garbles fails the run without Pi
//! having started, and the same holds for a launcher in its own process.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moebius_plugin_api::{
    Call, CommandLine, Host, Kind, LaunchAnswer, LaunchRequest, Manifest, Plugin, PluginError,
    Process, ProcessSpec, UrlsInside, Wrapped,
};
use serde_json::{Value, json};

use super::remote_tests::with_runner;
use super::testing::{child_pid, fake_pi, setup, wait_for, wait_until_ended, wait_until_gone};
use super::{PiBolt, lock};
use crate::board::{CardRuns, Variant};
use crate::harness::launch::Launcher;

/// How the fake launcher answers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unchanged,
    Wrap,
    Fails,
    Hangs,
    Garbles,
}

/// A launcher compiled in, which keeps every request it gets.
struct Fake {
    mode: Mode,
    dir: PathBuf,
    seen: Arc<Mutex<Vec<LaunchRequest>>>,
}

fn shell(script: &str, dir: &Path) -> CommandLine {
    CommandLine {
        program: "sh".into(),
        args: vec![
            "-c".into(),
            r#"echo "$0" >> "$1""#.into(),
            script.into(),
            dir.join("marker.txt").to_string_lossy().into_owned(),
        ],
        env: Default::default(),
    }
}

impl Plugin for Fake {
    fn manifest(&self) -> Manifest {
        Manifest {
            name: "box".into(),
            version: "0".into(),
            kinds: vec![Kind::Launcher],
        }
    }

    fn call(&self, call: Call) -> Result<Value, PluginError> {
        if call.method == "leftovers" {
            let leftover = self.dir.join("leftover.txt");
            return Ok(json!({ "remove": [leftover], "cleanup": [shell("swept", &self.dir)] }));
        }
        let request: LaunchRequest = serde_json::from_value(call.params)
            .map_err(|e| PluginError::BadParams(e.to_string()))?;
        lock(&self.seen).push(request.clone());
        let answer = match self.mode {
            Mode::Unchanged => LaunchAnswer::Unchanged {
                note: Some("not this one".into()),
            },
            Mode::Fails => return Err(PluginError::Failed("no sandbox here".into())),
            Mode::Hangs => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            Mode::Garbles => return Ok(json!({ "answer": "maybe" })),
            Mode::Wrap => {
                std::fs::write(self.dir.join("created.txt"), "profile\n").unwrap();
                let marker = self.dir.join("marker.txt");
                let mut args = vec![
                    "-c".to_string(),
                    r#"echo wrapped >> "$1"; shift; exec "$@""#.into(),
                    "sh".into(),
                    marker.to_string_lossy().into_owned(),
                    request.program,
                ];
                args.extend(request.args);
                LaunchAnswer::Wrapped(Box::new(Wrapped {
                    program: "sh".into(),
                    args,
                    env: request.env,
                    urls: UrlsInside {
                        router: Some("http://127.0.0.1:55".into()),
                        gateway: Some("http://127.0.0.1:55/gateway".into()),
                        check: Some("http://127.0.0.1:56".into()),
                    },
                    stop: Some(shell("stopped", &self.dir)),
                    cleanup: Some(shell("cleaned", &self.dir)),
                    remove: vec![self.dir.join("created.txt").to_string_lossy().into_owned()],
                    note: None,
                }))
            }
        };
        serde_json::to_value(answer).map_err(|e| PluginError::Failed(e.to_string()))
    }
}

/// Gives `pi` the launcher `mode`; the requests it gets are kept in the result.
fn with_launcher(pi: &mut PiBolt, dir: &Path, mode: Mode) -> Arc<Mutex<Vec<LaunchRequest>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    // Only a launcher that hangs is given a short time; a loaded machine must not fail the others.
    let limit = match mode {
        Mode::Hangs => Duration::from_secs(1),
        _ => Duration::from_secs(30),
    };
    let mut host = Host::new(limit);
    host.add(Fake {
        mode,
        dir: dir.to_path_buf(),
        seen: seen.clone(),
    });
    pi.launcher = Some(Arc::new(Launcher::new(Arc::new(host), "box")));
    seen
}

fn marker(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("marker.txt"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

fn run_dir(app: &crate::app::App, run: i64) -> PathBuf {
    app.data_dir.join(format!("runs/{run}"))
}

fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn an_unchanged_answer_starts_pi_as_without_a_launcher() {
    let (dir, app, mut pi, repo) = setup();
    let seen = with_launcher(&mut pi, dir.path(), Mode::Unchanged);
    let card = app
        .create_issue(repo, "Write done.txt", "", "person")
        .unwrap();
    let run = pi
        .start(&app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let run = wait_until_ended(&app, run.id);
    assert_eq!(run.status, "succeeded");
    // The launcher's note is an event of the run.
    let events = app.store.events(run.id).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "launcher_note" && e.data.to_string().contains("not this one")),
        "{events:?}"
    );

    // Started as asked: the fake saw the original environment and addresses.
    let env = json_file(&run_dir(&app, run.id).join("agent/env.json"));
    assert_eq!(env["MOEBIUS_CHECK_URL"], "http://127.0.0.1:9");
    assert!(marker(dir.path()).is_empty());

    let requests = lock(&seen);
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.run.id, run.id);
    assert_eq!(request.run.purpose, "code");
    assert_eq!(request.run.agent, "Coder");
    assert!(!request.run.chat);
    assert_eq!(request.run.card, Some(card.id));
    assert_eq!(request.program, fake_pi().to_string_lossy());
    assert!(request.args.windows(2).any(|w| w == ["--mode", "rpc"]));
    assert!(request.env["MOEBIUS_RUN_TOKEN"].starts_with("moebius-run-"));
    // The environment is the cleared one, not the server's.
    assert!(!request.env.contains_key("CARGO_MANIFEST_DIR"));
    assert_eq!(request.urls.router, "http://127.0.0.1:9");
    assert_eq!(request.urls.gateway, "http://127.0.0.1:9/mcp");
    assert_eq!(request.urls.check, "http://127.0.0.1:9");
    assert_eq!(request.runner, None);
    assert!(request.clone.is_some());
    // The worktree is read and written, the agent directory and the extensions only read.
    assert!(request.read_write.contains(&request.dir));
    assert!(request.read_write.iter().any(|p| p.ends_with("tmp")));
    assert!(request.read_write.iter().any(|p| p.ends_with("sessions")));
    assert!(request.read_only.iter().any(|p| p.ends_with("agent")));
    assert!(
        request
            .read_only
            .iter()
            .any(|p| p.ends_with("moebius-bridge.ts"))
    );
    assert!(request.read_only.iter().any(|p| p == "/agent/own.ts"));
    assert!(request.env["TMPDIR"].ends_with("tmp"));
}

#[test]
fn a_wrapped_launch_is_started_and_stopped_and_cleaned_up_as_the_launcher_says() {
    let (dir, app, mut pi, repo) = setup();
    let seen = with_launcher(&mut pi, dir.path(), Mode::Wrap);
    // An agent with an MCP server, whose gateway the launcher puts elsewhere.
    let servers = crate::harness::mcp::parse_file("[docs]\ncommand = \"docs\"\n", &[]).unwrap();
    let mut agent = pi.agent_set().agents["Coder"].clone();
    agent.mcp = servers
        .into_iter()
        .map(|(name, server)| (name, server.unwrap()))
        .collect();
    pi.agents
        .write()
        .unwrap()
        .agents
        .insert("Docs".into(), agent);
    let card = app
        .create_issue(repo, "Write done.txt", "", "person")
        .unwrap();
    let run = pi
        .start(&app, &card, "Docs", ("code", None), Variant::default())
        .unwrap();
    let run = wait_until_ended(&app, run.id);
    assert_eq!(run.status, "succeeded", "{run:?}");

    // Pi ran inside the wrapper, with the addresses the launcher named.
    let agent_dir = run_dir(&app, run.id).join("agent");
    assert_eq!(
        json_file(&agent_dir.join("env.json"))["MOEBIUS_CHECK_URL"],
        "http://127.0.0.1:56"
    );
    let models = json_file(&agent_dir.join("models.json"));
    assert_eq!(
        models["providers"]["moebius"]["baseUrl"],
        "http://127.0.0.1:55/v1"
    );
    let mcp = json_file(&agent_dir.join("mcp.json"));
    assert_eq!(
        mcp["mcpServers"]["moebius"]["url"],
        "http://127.0.0.1:55/gateway"
    );
    // The wrapper ran first, the stop command when Pi had ended, the cleanup command after it; the file the launcher
    // made is removed.
    wait_for("the cleanup command ran", || marker(dir.path()).len() == 3);
    assert_eq!(marker(dir.path()), ["wrapped", "stopped", "cleaned"]);
    assert!(!dir.path().join("created.txt").exists());
    assert_eq!(lock(&seen).len(), 1);
}

#[test]
fn a_run_that_is_stopped_uses_the_stop_command_and_is_cleaned_up_once() {
    let (dir, app, mut pi, repo) = setup();
    with_launcher(&mut pi, dir.path(), Mode::Wrap);
    // The fake never answers `abort`, so the stop kills it after the grace time.
    let card = app
        .create_issue(repo, "Please [deaf]", "", "person")
        .unwrap();
    let run = pi
        .start(&app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    wait_for("Pi runs inside the wrapper", || {
        marker(dir.path()) == ["wrapped"]
    });
    assert!(pi.stop(run.id));
    let run = wait_until_ended(&app, run.id);
    assert_eq!(run.status, "stopped", "{run:?}");
    wait_for("the cleanup command ran", || {
        marker(dir.path()).last().is_some_and(|l| l == "cleaned")
    });
    let lines = marker(dir.path());
    assert!(lines.contains(&"stopped".to_string()), "{lines:?}");
    assert_eq!(lines.iter().filter(|l| *l == "cleaned").count(), 1);
    assert!(!dir.path().join("created.txt").exists());
}

#[test]
fn a_server_restart_stops_a_wrapped_pi_with_the_stop_command_and_cleans_up() {
    let (dir, app, mut pi, repo) = setup();
    with_launcher(&mut pi, dir.path(), Mode::Wrap);
    let card = app
        .create_issue(repo, "Please [deaf]", "", "person")
        .unwrap();
    let run = pi
        .start(&app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let child = child_pid(&app, run.id);
    pi.kill_all();
    // The stop command ran and the cleanup after it, once; the reader of the run may run the stop again meanwhile.
    let lines = marker(dir.path());
    assert_eq!(lines[0], "wrapped", "{lines:?}");
    assert!(lines.contains(&"stopped".to_string()), "{lines:?}");
    assert_eq!(lines.iter().filter(|l| *l == "cleaned").count(), 1);
    assert!(
        lines.iter().position(|l| l == "stopped") < lines.iter().position(|l| l == "cleaned"),
        "{lines:?}"
    );
    assert!(!dir.path().join("created.txt").exists());
    wait_until_gone("the process Pi started", child);
    wait_until_ended(&app, run.id);
    // The reader finds the cleanup done and does not run it again.
    let lines = marker(dir.path());
    assert_eq!(
        lines.iter().filter(|l| *l == "cleaned").count(),
        1,
        "{lines:?}"
    );
}

#[test]
fn a_run_on_a_runner_is_told_to_the_launcher_and_an_unchanged_answer_starts_it_over_ssh() {
    let (dir, app, mut pi, repo, _remote, key) = with_runner();
    let seen = with_launcher(&mut pi, dir.path(), Mode::Unchanged);
    let card = app
        .create_issue(repo, "Write done.txt", "", "person")
        .unwrap();
    let run = pi
        .start(&app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let run = wait_until_ended(&app, run.id);
    assert_eq!(run.status, "succeeded", "{run:?}");
    assert_eq!(lock(&seen)[0].runner.as_deref(), Some("box"));
    assert!(std::fs::read_to_string(format!("{}.log", key.display())).is_ok());
}

#[test]
fn a_launcher_that_wraps_a_run_on_a_runner_fails_the_run_and_is_cleaned_up_after() {
    let (dir, app, mut pi, repo, _remote, _key) = with_runner();
    with_launcher(&mut pi, dir.path(), Mode::Wrap);
    let card = app
        .create_issue(repo, "Write done.txt", "", "person")
        .unwrap();
    let started = pi.start(&app, &card, "Coder", ("code", None), Variant::default());
    let runs = app.store.runs_for_card(card.id).unwrap();
    assert!(
        started.is_err() || runs.iter().all(|r| r.status == "failed"),
        "{started:?} {runs:?}"
    );
    assert!(lock(&pi.live).is_empty());
    assert_eq!(marker(dir.path()), ["cleaned"]);
    assert!(!dir.path().join("created.txt").exists());
}

#[test]
fn a_chat_given_another_model_is_asked_of_the_launcher_again() {
    let (dir, app, mut pi, _repo) = setup();
    let seen = with_launcher(&mut pi, dir.path(), Mode::Wrap);
    let chat = crate::board::ChatRuns::start_chat(
        &pi,
        &app,
        "Coder",
        (None, false),
        None,
        &[],
        None,
        "chat",
        &crate::app::ChatSettings::default(),
    )
    .unwrap();
    wait_for("the chat waits", || {
        app.run(chat.id).unwrap().status == "waiting"
    });
    app.change_chat_settings(&pi, chat.id, (Some("other-model"), None, &[]))
        .unwrap();
    // The first Pi was stopped and cleaned up, and the launcher was asked again for the one that took its place.
    assert_eq!(lock(&seen).len(), 2);
    wait_for("the second Pi is wrapped", || {
        marker(dir.path()) == ["wrapped", "stopped", "cleaned", "wrapped"]
    });
    assert!(crate::board::ChatRuns::end(&pi, chat.id));
    wait_until_ended(&app, chat.id);
}

#[test]
fn a_chat_taken_up_again_is_asked_of_the_launcher_again() {
    let (dir, app, mut pi, _repo) = setup();
    let seen = with_launcher(&mut pi, dir.path(), Mode::Wrap);
    let chat = crate::board::ChatRuns::start_chat(
        &pi,
        &app,
        "Coder",
        (None, false),
        None,
        &[],
        None,
        "chat",
        &crate::app::ChatSettings::default(),
    )
    .unwrap();
    wait_for("the chat waits", || {
        app.run(chat.id).unwrap().status == "waiting"
    });
    assert!(crate::board::CardRuns::stop(&pi, chat.id));
    assert_eq!(wait_until_ended(&app, chat.id).status, "stopped");
    assert_eq!(lock(&seen).len(), 1);

    app.say_in_chat(&pi, chat.id, "Carry on", &[], None)
        .unwrap();
    // The agent made again is wrapped like the first, never started bare.
    assert_eq!(lock(&seen).len(), 2);
    wait_for("the second Pi is wrapped", || {
        let marks = marker(dir.path());
        marks.first().map(String::as_str) == Some("wrapped")
            && marks.iter().filter(|m| *m == "wrapped").count() == 2
    });
    wait_for("the chat waits", || {
        app.run(chat.id).unwrap().status == "waiting"
    });
    assert!(crate::board::ChatRuns::end(&pi, chat.id));
    wait_until_ended(&app, chat.id);
}

#[test]
fn a_launcher_that_fails_hangs_or_garbles_fails_the_run_and_pi_never_starts() {
    for mode in [Mode::Fails, Mode::Hangs, Mode::Garbles] {
        let (dir, app, mut pi, repo) = setup();
        let seen = with_launcher(&mut pi, dir.path(), mode);
        let card = app
            .create_issue(repo, "Write done.txt", "", "person")
            .unwrap();
        let started = pi.start(&app, &card, "Coder", ("code", None), Variant::default());
        assert_eq!(lock(&seen).len(), 1);
        // Nothing runs and nothing is waiting for Pi.
        assert!(lock(&pi.live).is_empty());
        let runs = app.store.runs_for_card(card.id).unwrap();
        assert!(
            started.is_err() || runs.iter().all(|r| r.status == "failed"),
            "{started:?} {runs:?}"
        );
        for run in &runs {
            let run = wait_until_ended(&app, run.id);
            assert_eq!(run.status, "failed");
            assert!(
                !run_dir(&app, run.id).join("agent/argv.json").exists(),
                "Pi started without the launcher"
            );
        }
        assert!(started.is_err() || !runs.is_empty());
        assert!(marker(dir.path()).is_empty());
    }
}

#[test]
fn a_chat_is_told_to_the_launcher_as_one_and_has_no_terminal_interface() {
    let (dir, app, mut pi, _repo) = setup();
    let seen = with_launcher(&mut pi, dir.path(), Mode::Wrap);
    let chat = crate::board::ChatRuns::start_chat(
        &pi,
        &app,
        "Coder",
        (None, false),
        None,
        &[],
        None,
        "chat",
        &crate::app::ChatSettings::default(),
    )
    .unwrap();
    assert!(lock(&seen)[0].run.chat);
    assert_eq!(lock(&seen)[0].run.purpose, "chat");
    // The terminal interface would be a Pi outside the launcher.
    let tui = crate::board::ChatRuns::tui(&pi, &app, chat.id);
    assert!(tui.unwrap_err().contains("launcher"));
    assert!(crate::board::ChatRuns::end(&pi, chat.id));
    wait_until_ended(&app, chat.id);
    wait_for("the cleanup command ran", || {
        marker(dir.path()).last().is_some_and(|l| l == "cleaned")
    });
}

/// The launcher of `testdata/fake-launcher`, a Python program that speaks the process protocol.
fn process_launcher(dir: &Path) -> Arc<Launcher> {
    let spec = ProcessSpec {
        name: "box".into(),
        program: Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fake-launcher"),
        args: vec![dir.to_string_lossy().into_owned()],
        dir: dir.to_path_buf(),
    };
    let process = Process::start(spec, Duration::from_secs(20), |_| {});
    let mut host = Host::new(Duration::from_secs(20));
    host.add_process(process);
    Arc::new(Launcher::new(Arc::new(host), "box"))
}

#[test]
fn a_launcher_in_its_own_process_wraps_a_run_and_names_its_leftovers() {
    let (dir, app, mut pi, repo) = setup();
    let launcher = process_launcher(dir.path());
    // What an earlier run left is removed when the server starts.
    std::fs::write(dir.path().join("leftover.txt"), "old profile").unwrap();
    launcher.clean_leftovers(&[]).unwrap();
    assert!(!dir.path().join("leftover.txt").exists());

    pi.launcher = Some(launcher);
    let card = app
        .create_issue(repo, "Write done.txt", "", "person")
        .unwrap();
    let run = pi
        .start(&app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let run = wait_until_ended(&app, run.id);
    assert_eq!(run.status, "succeeded", "{run:?}");
    wait_for("the cleanup command ran", || {
        marker(dir.path()).last().is_some_and(|l| l == "cleaned")
    });
    assert_eq!(marker(dir.path()), ["wrapped", "stopped", "cleaned"]);
    assert!(!dir.path().join("created.txt").exists());
    let env = json_file(&run_dir(&app, run.id).join("agent/env.json"));
    assert_eq!(env["MOEBIUS_CHECK_URL"], "http://127.0.0.1:56");
    // The request was the protocol's: a launch with the run and the program.
    let requests = std::fs::read_to_string(dir.path().join("requests.jsonl")).unwrap();
    let launch: Value = serde_json::from_str(requests.lines().nth(1).unwrap()).unwrap();
    assert_eq!(launch["method"], "launch");
    assert_eq!(launch["params"]["run"]["purpose"], "code");
    assert_eq!(
        launch["params"]["program"].as_str().unwrap(),
        fake_pi().to_string_lossy()
    );
}

#[test]
fn a_launcher_process_that_is_gone_fails_the_run_instead_of_starting_it_bare() {
    let (dir, app, mut pi, repo) = setup();
    // A program that is no launcher: it exits at once.
    let spec = ProcessSpec {
        name: "box".into(),
        program: "false".into(),
        args: vec![],
        dir: dir.path().to_path_buf(),
    };
    let mut host = Host::new(Duration::from_secs(1));
    host.add_process(Process::start(spec, Duration::from_secs(5), |_| {}));
    pi.launcher = Some(Arc::new(Launcher::new(Arc::new(host), "box")));
    let card = app
        .create_issue(repo, "Write done.txt", "", "person")
        .unwrap();
    let started = pi.start(&app, &card, "Coder", ("code", None), Variant::default());
    for run in app.store.runs_for_card(card.id).unwrap() {
        assert!(!run_dir(&app, run.id).join("agent/argv.json").exists());
    }
    assert!(
        started.is_err()
            || app
                .store
                .runs_for_card(card.id)
                .unwrap()
                .iter()
                .all(|r| r.status == "failed")
    );
}
