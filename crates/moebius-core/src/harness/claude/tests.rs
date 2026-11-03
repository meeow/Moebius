//! The Claude Code harness against a fake `claude` (`testdata/fake-claude`): its command line and environment, the run
//! events its records become, how its runs and chats end, and the router's check of its tool calls.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use moebius_plugin_api::{Call, DecideRequest, Host, Kind, Manifest, Plugin, PluginError};
use serde_json::{Value, json};

use super::*;
use crate::config::{AgentConfig, AgentSet};
use crate::decide::check::Checker;
use crate::repos::git;
use crate::router::{Protocol, Router, Upstream};
use crate::store::Store;

/// Refuses `bash`, naming a rule, and allows the rest.
struct NoBash;
impl Plugin for NoBash {
    fn manifest(&self) -> Manifest {
        Manifest {
            name: "no-bash".into(),
            version: "0".into(),
            kinds: vec![Kind::Decider],
        }
    }
    fn call(&self, call: Call) -> Result<Value, PluginError> {
        let r: DecideRequest = serde_json::from_value(call.params).unwrap();
        Ok(if r.context["tool"] == "bash" {
            json!({"choice": "refuse", "reason": "rule no-bash: no shell"})
        } else {
            json!({"choice": "allow", "reason": "fine"})
        })
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    claude: ClaudeCode,
    repo: i64,
}

/// A core with one repository, whose `tool_call` decider refuses `bash`, and Claude Code as the fake with the Coder on
/// the anthropic protocol; the router is at `router_url`.
fn setup_with(router: Option<(Arc<Router>, String)>) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "--quiet", "-b", "main"]).unwrap();
    std::fs::write(origin.join("README.md"), "hello\n").unwrap();
    git(&origin, &["add", "."]).unwrap();
    git(&origin, &["commit", "--quiet", "-m", "first"]).unwrap();
    let store = Arc::new(Store::in_memory(&dir.path().join("blobs")).unwrap());
    let mut host = Host::new(Duration::from_secs(5));
    host.add(NoBash);
    let app = App::new(store.clone(), Arc::new(host), &dir.path().join("data"));
    app.decisions.route("tool_call", "no-bash");
    let app = Arc::new(app);
    let repo = app
        .add_repository("demo", origin.to_str().unwrap())
        .unwrap();
    let skill = dir.path().join("house-style");
    std::fs::create_dir_all(&skill).unwrap();
    let coder = AgentConfig {
        model: "claude-test".into(),
        protocol: Protocol::Anthropic,
        thinking: None,
        instructions: Some("Be brief.".into()),
        skills: vec![skill],
        extensions: vec![],
        mcp: BTreeMap::new(),
        token_budget: None,
        harness: HarnessKind::ClaudeCode,
        tool_output: Default::default(),
        engineering_skills: true,
        runner: None,
    };
    let (router, url) = router.unwrap_or_else(|| {
        (
            Arc::new(Router::new(store, vec![])),
            "http://127.0.0.1:9".into(),
        )
    });
    router.set_checker(Checker::new(app.clone(), None));
    let agents = AgentSet {
        agents: BTreeMap::from([("Coder".to_string(), coder)]),
        errors: BTreeMap::new(),
        skill_overrides: BTreeMap::new(),
    };
    let claude = ClaudeCode::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fake-claude"),
        "/usr/local/bin/moebius".into(),
        url,
        router,
        Arc::new(RwLock::new(agents)),
    );
    Setup {
        _dir: dir,
        app,
        claude,
        repo: repo.id,
    }
}

fn setup() -> Setup {
    setup_with(None)
}

fn wait_for_run(app: &App, run_id: i64, what: &str, holds: impl Fn(&Run) -> bool) -> Run {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let run = app.store.run(run_id).unwrap().unwrap();
        if holds(&run) {
            return run;
        }
        let kinds: Vec<String> = app
            .store
            .events(run_id)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(Instant::now() < deadline, "{what}: {run:?} {kinds:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_until_ended(app: &App, run_id: i64) -> Run {
    wait_for_run(app, run_id, "the run ends", |run| run.ended_at.is_some())
}

/// The file `name` the fake wrote into the run's configuration directory.
fn written(app: &App, run_id: i64, name: &str) -> String {
    let file = app.data_dir.join(format!("runs/{run_id}/claude/{name}"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !file.exists() {
        assert!(
            Instant::now() < deadline,
            "{} is not written",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::read_to_string(file).unwrap()
}

fn kinds(app: &App, run_id: i64) -> Vec<String> {
    app.store
        .events(run_id)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

#[test]
fn a_code_run_on_claude_code_is_given_the_agent_and_the_router_and_commits_its_work() {
    let s = setup();
    let card = s
        .app
        .create_issue(s.repo, "Write done.txt", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    assert_eq!(run.harness, "claude-code");
    let argv: Vec<String> = serde_json::from_str(&written(&s.app, run.id, "argv.json")).unwrap();
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "succeeded", "{:?}", kinds(&s.app, run.id));
    assert_eq!(run.summary.as_deref(), Some("Done."));

    for pair in [
        ["--input-format", "stream-json"],
        ["--output-format", "stream-json"],
        ["--model", "claude-test"],
        ["--append-system-prompt", "Be brief."],
        ["--permission-mode", "bypassPermissions"],
    ] {
        assert!(argv.windows(2).any(|w| w == pair), "{pair:?} in {argv:?}");
    }
    assert!(argv.contains(&"-p".to_string()) && argv.contains(&"--verbose".to_string()));
    assert!(argv.contains(&"--session-id".to_string()));
    assert!(!argv.contains(&"--mcp-config".to_string()), "{argv:?}");
    let env: Value = serde_json::from_str(&written(&s.app, run.id, "env.json")).unwrap();
    // The server's environment does not reach Claude Code (cargo sets this variable for the test), but its PATH does.
    assert!(env["CARGO_MANIFEST_DIR"].is_null(), "{env}");
    assert!(env["PATH"].is_string());
    assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:9");
    let token = env["ANTHROPIC_API_KEY"].as_str().unwrap();
    assert!(token.starts_with("moebius-run-"));
    assert_eq!(env["MOEBIUS_RUN_TOKEN"], token);
    assert!(env["ANTHROPIC_AUTH_TOKEN"].is_null());
    // The token is withdrawn once the run has ended.
    assert_eq!(s.claude.router.token_count(), 0);
    // The agent's skill is in the run's own configuration directory.
    let skill = s
        .app
        .data_dir
        .join(format!("runs/{}/claude/skills/house-style", run.id));
    assert!(skill.is_dir());
    let prompt = written(&s.app, run.id, "prompt.txt");
    assert!(prompt.starts_with("Work on issue #1: Write done.txt"));

    // Its records are the events of a Pi-Bolt run.
    let kinds = kinds(&s.app, run.id);
    for kind in [
        "agent_start",
        "claude_system",
        "message_update",
        "tool_execution_start",
        "tool_execution_end",
        "agent_end",
        "agent_settled",
    ] {
        assert!(kinds.contains(&kind.to_string()), "{kind} in {kinds:?}");
    }
    let clone = PathBuf::from(&s.app.store.repository(s.repo).unwrap().unwrap().path);
    let shown = git(&clone, &["show", &format!("{}:done.txt", run.branch())]).unwrap();
    assert_eq!(shown, "done");
}

#[test]
fn a_run_ends_as_claude_code_says() {
    let s = setup();
    let card = s
        .app
        .create_issue(s.repo, "Do [nothing]", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "failed");
    let last = s.app.store.events(run.id).unwrap().pop().unwrap();
    assert_eq!(last.data["message"], "the agent changed nothing");

    // A review changes nothing, and that is no failure.
    let review = s
        .claude
        .start(&s.app, &card, "Coder", ("review", None), Variant::default())
        .unwrap();
    let review = wait_until_ended(&s.app, review.id);
    assert_eq!(review.status, "succeeded");
    assert_eq!(
        review.summary.as_deref(),
        Some("Looks fine.\nVERDICT: approve")
    );

    // A result that is an error fails the run.
    let card = s
        .app
        .create_issue(s.repo, "Please [fail]", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "failed");
    let last = s.app.store.events(run.id).unwrap().pop().unwrap();
    assert!(
        last.data["message"]
            .as_str()
            .unwrap()
            .contains("overloaded")
    );
}

#[test]
fn a_run_is_steered_with_a_message_and_stopped_by_killing_it() {
    let s = setup();
    let card = s
        .app
        .create_issue(s.repo, "Please [hang]", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    written(&s.app, run.id, "prompt.txt");
    // The steer is the next user message, which the fake does not answer either.
    assert!(s.claude.steer(run.id, "Look at the tests. [hang]"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !written(&s.app, run.id, "stdin.jsonl").contains("Look at the tests.") {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let stdin = written(&s.app, run.id, "stdin.jsonl");
    let steer: Value = serde_json::from_str(stdin.lines().last().unwrap()).unwrap();
    assert_eq!(steer["type"], "user");
    assert_eq!(
        steer["message"]["content"][0]["text"],
        "Look at the tests. [hang]"
    );

    assert!(s.claude.stop(run.id));
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "stopped");
    assert!(!s.claude.stop(run.id));
    assert!(!s.claude.steer(run.id, "Too late."));
}

// Linux only: whether the fake's `sleep` runs is read from `/proc`.
#[cfg(target_os = "linux")]
#[test]
fn stopping_a_run_kills_what_its_agent_started() {
    let s = setup();
    let card = s
        .app
        .create_issue(s.repo, "Please [child]", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let child: u32 = written(&s.app, run.id, "child.txt").parse().unwrap();
    assert!(crate::platform::is_running(child));
    assert!(s.claude.stop(run.id));
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "stopped");
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::platform::is_running(child) {
        assert!(Instant::now() < deadline, "the background process runs");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Linux only: whether the fake's `sleep` runs is read from `/proc`.
#[cfg(target_os = "linux")]
#[test]
fn a_chat_that_ends_leaves_no_background_process_of_its_agent() {
    let s = setup();
    let repository = s.app.repository(s.repo).unwrap();
    let run = s
        .claude
        .start_chat(
            &s.app,
            "Coder",
            (Some(&repository), false),
            Some("Hello"),
            &[],
            Some("[child] Hello"),
            "chat",
            &ChatSettings::default(),
        )
        .unwrap();
    let child: u32 = written(&s.app, run.id, "child.txt").parse().unwrap();
    assert!(crate::platform::is_running(child));
    // The fake hangs on that message; the next one is answered, and the chat waits.
    s.app
        .say_in_chat(&s.claude, run.id, "Next", &[], None)
        .unwrap();
    wait_for_run(&s.app, run.id, "the agent answers", |run| {
        run.status == "waiting"
    });
    // Claude Code exits on its own once its stdin is closed; what it started must not stay.
    s.app.end_chat(&s.claude, run.id).unwrap();
    wait_until_ended(&s.app, run.id);
    let deadline = Instant::now() + Duration::from_secs(20);
    while crate::platform::is_running(child) {
        assert!(Instant::now() < deadline, "the background process runs");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Linux only: the processes of the run are found by their environment in `/proc`.
#[cfg(target_os = "linux")]
#[test]
fn a_launch_that_fails_after_claude_code_started_kills_it() {
    let s = setup();
    // The chat cannot be marked as waiting once Claude Code runs.
    s.app.store.execute_batch_for_test(
        "CREATE TRIGGER no_waiting BEFORE UPDATE OF status ON runs WHEN NEW.status = 'waiting' \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    );
    let started = crate::board::ChatRuns::start_chat(
        &s.claude,
        &s.app,
        "Coder",
        (None, false),
        None,
        &[],
        None,
        "chat",
        &ChatSettings::default(),
    );
    assert!(started.is_err());
    assert!(lock(&s.claude.live).is_empty(), "Claude Code stays in live");
    let marker = s.app.data_dir.join("runs").to_string_lossy().into_owned();
    assert!(
        crate::harness::pibolt::testing::processes_with_env(&marker).is_empty(),
        "an orphaned Claude Code runs"
    );
}

// Unix only: the fake `claude` is a script; it must not even be started.
#[cfg(unix)]
#[test]
fn claude_code_does_not_start_while_a_launcher_is_configured() {
    let mut s = setup();
    s.claude.launcher_configured = true;
    let started = crate::board::ChatRuns::start_chat(
        &s.claude,
        &s.app,
        "Coder",
        (None, false),
        None,
        &[],
        None,
        "chat",
        &ChatSettings::default(),
    );
    assert!(started.is_err_and(|e| e.to_string().contains("launcher")));
    assert!(lock(&s.claude.live).is_empty());
}

#[test]
fn a_chat_given_another_model_before_its_first_message_starts_a_session_instead_of_resuming_one() {
    let s = setup();
    let repository = s.app.repository(s.repo).unwrap();
    let run = s
        .claude
        .start_chat(
            &s.app,
            "Coder",
            (Some(&repository), false),
            None,
            &[],
            None,
            "chat",
            &ChatSettings::default(),
        )
        .unwrap();
    let argv_of = |what: &str| -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let argv: Vec<String> =
                serde_json::from_str(&written(&s.app, run.id, "argv.json")).unwrap_or_default();
            if argv.windows(2).any(|w| w == ["--model", what]) {
                return argv;
            }
            assert!(Instant::now() < deadline, "no claude with {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let first = argv_of("claude-test");
    let session = first[first.iter().position(|a| a == "--session-id").unwrap() + 1].clone();

    // Nothing was said yet: Claude Code has written no session to resume.
    let served = [Protocol::Anthropic];
    s.app
        .change_chat_settings(&s.claude, run.id, (Some("claude-other"), None, &served))
        .unwrap();
    let second = argv_of("claude-other");
    assert!(
        second
            .windows(2)
            .any(|w| w == ["--session-id", session.as_str()])
    );
    assert!(!second.contains(&"--resume".to_string()), "{second:?}");

    // After a message it is resumed.
    wait_for_run(&s.app, run.id, "the chat waits", |run| {
        run.status == "waiting"
    });
    s.app
        .say_in_chat(&s.claude, run.id, "Hello", &[], None)
        .unwrap();
    let answered = |run: &Run| {
        run.status == "waiting"
            && s.app
                .store
                .events(run.id)
                .unwrap()
                .iter()
                .any(|e| e.kind == "agent_settled")
    };
    wait_for_run(&s.app, run.id, "the agent answers", answered);
    s.app
        .change_chat_settings(&s.claude, run.id, (Some("claude-third"), None, &served))
        .unwrap();
    let third = argv_of("claude-third");
    assert!(
        third
            .windows(2)
            .any(|w| w == ["--resume", session.as_str()])
    );

    s.app.end_chat(&s.claude, run.id).unwrap();
    wait_until_ended(&s.app, run.id);
}

#[test]
fn an_agent_that_does_not_speak_anthropic_cannot_run_on_claude_code() {
    let s = setup();
    s.claude
        .agents
        .write()
        .unwrap()
        .agents
        .get_mut("Coder")
        .unwrap()
        .protocol = Protocol::OpenAi;
    let card = s
        .app
        .create_issue(s.repo, "Anything", "", "person")
        .unwrap();
    let error = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap_err();
    assert!(
        matches!(error, StartError::UnknownAgent(ref why) if why.contains("anthropic")),
        "{error}"
    );
    assert!(s.app.store.runs_for_card(card.id).unwrap().is_empty());
}

#[test]
fn a_chat_on_claude_code_waits_between_messages_resumes_its_session_for_another_model_and_ends() {
    let s = setup();
    let repository = s.app.repository(s.repo).unwrap();
    let run = s
        .claude
        .start_chat(
            &s.app,
            "Coder",
            (Some(&repository), false),
            Some("Hello"),
            &[],
            Some("Hello"),
            "chat",
            &ChatSettings::default(),
        )
        .unwrap();
    let waiting = |run: &Run| run.status == "waiting";
    wait_for_run(&s.app, run.id, "the agent answers", waiting);
    assert_eq!(run.harness, "claude-code");
    let first: Vec<String> = serde_json::from_str(&written(&s.app, run.id, "argv.json")).unwrap();
    let session = first[first.iter().position(|a| a == "--session-id").unwrap() + 1].clone();

    s.app
        .say_in_chat(&s.claude, run.id, "[write notes.txt] Again", &[], None)
        .unwrap();
    let answered = |run: &Run| {
        run.status == "waiting"
            && s.app
                .store
                .events(run.id)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "agent_settled")
                .count()
                == 2
    };
    wait_for_run(&s.app, run.id, "the agent answers again", answered);

    // Another model: a new Claude Code that resumes the session.
    let served = [Protocol::Anthropic];
    s.app
        .change_chat_settings(&s.claude, run.id, (Some("claude-other"), None, &served))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let argv = loop {
        let argv: Vec<String> =
            serde_json::from_str(&written(&s.app, run.id, "argv.json")).unwrap();
        if argv.contains(&"--resume".to_string()) {
            break argv;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(argv.windows(2).any(|w| w == ["--resume", session.as_str()]));
    assert!(argv.windows(2).any(|w| w == ["--model", "claude-other"]));
    assert_eq!(s.app.run(run.id).unwrap().status, "waiting");

    s.app.end_chat(&s.claude, run.id).unwrap();
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "succeeded");
    assert!(!s.claude.say(run.id, "Hello?", &[]));
    // What it wrote is on the chat's branch.
    let clone = PathBuf::from(&repository.path);
    let shown = git(
        &clone,
        &["show", &format!("moebius/chat-{}:notes.txt", run.id)],
    );
    assert_eq!(shown.unwrap(), "written");
}

#[test]
fn a_chat_on_claude_code_whose_agent_went_away_is_taken_up_again_with_what_was_said() {
    let s = setup();
    let repository = s.app.repository(s.repo).unwrap();
    let run = s
        .claude
        .start_chat(
            &s.app,
            "Coder",
            (Some(&repository), false),
            Some("Hello"),
            &[],
            Some("Hello"),
            "chat",
            &ChatSettings::default(),
        )
        .unwrap();
    wait_for_run(&s.app, run.id, "the agent answers", |run| {
        run.status == "waiting"
    });
    s.app
        .say_in_chat(&s.claude, run.id, "[write notes.txt] Again", &[], None)
        .unwrap();
    wait_for_run(&s.app, run.id, "the agent answers again", |run| {
        run.status == "waiting"
            && s.app
                .store
                .events(run.id)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "agent_settled")
                .count()
                == 2
    });
    assert!(s.claude.stop(run.id));
    let ended = wait_until_ended(&s.app, run.id);
    assert_eq!(ended.status, "stopped");

    let again = s
        .app
        .say_in_chat(&s.claude, run.id, "Carry on", &[], None)
        .unwrap();
    assert_eq!(again.id, run.id, "the same chat");
    assert_eq!(again.ended_at, None);
    wait_for_run(&s.app, run.id, "the agent answers", |run| {
        run.status == "waiting"
            && s.app
                .store
                .events(run.id)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "agent_settled")
                .count()
                == 3
    });
    // The new Claude Code was told what was said, on a session of its own.
    let argv: Vec<String> = serde_json::from_str(&written(&s.app, run.id, "argv.json")).unwrap();
    assert!(argv.contains(&"--session-id".to_string()), "{argv:?}");
    assert!(!argv.contains(&"--resume".to_string()), "{argv:?}");
    let told = written(&s.app, run.id, "stdin.jsonl");
    assert!(
        told.contains("This chat continues an earlier one"),
        "{told}"
    );
    assert!(told.contains("Carry on"), "{told}");
    assert!(s.claude.stop(run.id));
    wait_until_ended(&s.app, run.id);
}

/// An Anthropic upstream that answers every request with a text and a call of `bash`.
async fn upstream_calling_bash() -> String {
    use axum::routing::post;
    let app = axum::Router::new().route(
        "/v1/messages",
        post(|| async {
            axum::Json(json!({
                "model": "claude-test", "stop_reason": "tool_use",
                "content": [{"type": "text", "text": "Cleaning up."},
                            {"type": "tool_use", "id": "toolu_9", "name": "bash", "input": {"command": "rm -rf /"}}],
                "usage": {"input_tokens": 5, "output_tokens": 3}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_router_refuses_a_tool_call_of_claude_code_before_it_sees_it() {
    let upstream = upstream_calling_bash().await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::in_memory(&dir.path().join("blobs")).unwrap());
    let router = Arc::new(Router::new(
        store,
        vec![Upstream {
            protocol: Protocol::Anthropic,
            base_url: upstream,
            api_key: None,
            oauth: None,
        }],
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let routes = router.clone().routes();
    tokio::spawn(async move { axum::serve(listener, routes).await });

    let s = tokio::task::spawn_blocking(move || {
        let s = setup_with(Some((router, url)));
        let card = s
            .app
            .create_issue(s.repo, "Clean up [router]", "", "person")
            .unwrap();
        let run = s
            .claude
            .start(&s.app, &card, "Coder", ("review", None), Variant::default())
            .unwrap();
        let run = wait_until_ended(&s.app, run.id);
        (s, run)
    });
    let (s, run) = s.await.unwrap();
    assert_eq!(run.status, "succeeded");
    // The harness saw text where the call was, naming the rule, and never the call.
    let summary = run.summary.unwrap();
    assert!(
        summary.contains("Moebius refused the tool call bash (toolu_9): rule no-bash: no shell"),
        "{summary}"
    );
    let events = s.app.store.events(run.id).unwrap();
    assert!(!events.iter().any(|e| e.kind == "tool_execution_start"));
    let check = events.iter().find(|e| e.kind == "tool_check").unwrap();
    assert_eq!(check.data["decision"], "refuse");
    assert_eq!(check.data["tool_call_id"], "toolu_9");
}

/// A server whose Coder goes to the runner `box`, whose "machine" is the directory `remote` and whose `ssh` is the
/// fake one of `testdata`.
#[cfg(unix)] // The fake `ssh` runs the command in `sh`.
fn with_runner() -> (Setup, PathBuf) {
    let mut s = setup();
    let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
    let remote = s._dir.path().join("remote");
    let config = crate::config::RunnerConfig {
        name: "box".into(),
        host: "gpu.test".into(),
        user: None,
        port: None,
        key_file: None,
        working_dir: remote.to_string_lossy().into_owned(),
        max_runs: 1,
        bin_dir: Some(testdata.to_string_lossy().into_owned()),
        ssh: testdata.join("fake-ssh"),
    };
    s.claude.runners = Arc::new(crate::harness::remote::Runners::new(vec![config]));
    {
        let mut agents = s.claude.agents.write().unwrap();
        let coder = agents.agents.get_mut("Coder").unwrap();
        coder.runner = Some("box".into());
        // The built-in skills are not written to the data directory by this test, and `tar` cannot ship a dangling link.
        coder.engineering_skills = false;
    }
    (s, remote)
}

/// The file the fake Claude of a run on the runner wrote into its configuration directory there.
#[cfg(unix)] // See `with_runner`.
fn remote_file(s: &Setup, remote: &Path, run_id: i64, name: &str) -> PathBuf {
    let config = s.app.data_dir.join(format!("runs/{run_id}/claude"));
    remote
        .join(format!("runs/{run_id}/files"))
        .join(config.strip_prefix("/").unwrap())
        .join(name)
}

#[cfg(unix)] // See `with_runner`.
#[test]
fn a_code_run_on_claude_code_works_on_a_runner_and_its_commits_come_back() {
    let (s, remote) = with_runner();
    let card = s
        .app
        .create_issue(s.repo, "Write done.txt", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    assert_eq!(run.runner.as_deref(), Some("box"));
    let run = wait_until_ended(&s.app, run.id);
    assert_eq!(run.status, "succeeded", "{:?}", kinds(&s.app, run.id));
    let clone = PathBuf::from(&s.app.store.repository(s.repo).unwrap().unwrap().path);
    let shown = git(&clone, &["show", &format!("{}:done.txt", run.branch())]).unwrap();
    assert_eq!(shown, "done");
    assert!(!remote.join(format!("runs/{}", run.id)).exists());
}

#[cfg(unix)] // See `with_runner`.
#[test]
fn killing_all_claude_code_runs_closes_their_tunnels_and_clears_the_runner() {
    use std::net::TcpStream;
    let (s, remote) = with_runner();
    let card = s
        .app
        .create_issue(s.repo, "Please [hang]", "", "person")
        .unwrap();
    let run = s
        .claude
        .start(&s.app, &card, "Coder", ("code", None), Variant::default())
        .unwrap();
    let env_file = remote_file(&s, &remote, run.id, "env.json");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !env_file.is_file() {
        assert!(
            Instant::now() < deadline,
            "the fake Claude writes its environment"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let env: Value = serde_json::from_slice(&std::fs::read(&env_file).unwrap()).unwrap();
    let port: u16 = env["ANTHROPIC_BASE_URL"]
        .as_str()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

    s.claude.kill_all();
    assert!(!remote.join(format!("runs/{}", run.id)).exists());
    let deadline = Instant::now() + Duration::from_secs(20);
    while TcpStream::connect(("127.0.0.1", port)).is_ok() {
        assert!(Instant::now() < deadline, "the tunnel is closed");
        std::thread::sleep(Duration::from_millis(20));
    }
}
