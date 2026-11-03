//! The harness the server drives (`Harnesses`): Pi-Bolt and Claude Code side by side, sharing the agents, with each run
//! started on the harness its agent's `harness` names. A built-in agent without its own configuration runs on the
//! Coder's harness. `Harnesses` only looks the harness up, by agent when a run starts and by the stored run when
//! something is done to a running one; the lifecycle itself is shared (see `run`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::app::{App, ChatSettings};
use crate::board::{AgentInfo, CardRuns, ChatRuns, ImproveRuns, Runs, StartError, Variant};
use crate::config::{AgentConfig, AgentSet, HarnessKind};
use crate::harness::claude::ClaudeCode;
use crate::harness::pibolt::{
    AUTORESEARCHER_AGENT, PiBolt, RETRO_AGENT, agent_list, has_agent, resolve_agent,
};
use crate::store::{Card, Repository, Run};

/// Pi-Bolt and Claude Code, picked by agent.
pub struct Harnesses {
    pub pi_bolt: Arc<PiBolt>,
    pub claude_code: Arc<ClaudeCode>,
    /// The agents both run. The harness configuration repository swaps them when it changes.
    pub agents: Arc<RwLock<AgentSet>>,
}

impl Harnesses {
    /// Both harnesses, with Claude Code started as `claude_code` and sharing what `pi_bolt` has: its agents, the router
    /// and the `moebius` executable.
    pub fn new(pi_bolt: PiBolt, claude_code: &Path) -> Self {
        let agents = pi_bolt.agents.clone();
        let mut claude = ClaudeCode::new(
            claude_code.to_path_buf(),
            pi_bolt.moebius.clone(),
            pi_bolt.router_url.clone(),
            pi_bolt.router.clone(),
            agents.clone(),
        );
        claude.runners = pi_bolt.runners.clone();
        claude.launcher_configured = pi_bolt.launcher.is_some();
        Self {
            pi_bolt: Arc::new(pi_bolt),
            claude_code: Arc::new(claude),
            agents,
        }
    }

    /// Tells both harnesses the desktop API's base URL, once the server listens.
    pub fn set_api_url(&self, url: String) {
        let _ = self.claude_code.api_url.set(url.clone());
        let _ = self.pi_bolt.api_url.set(url);
    }

    /// Kills every run's process, for a server that is about to restart.
    pub fn kill_all(&self) {
        self.pi_bolt.kill_all();
        self.claude_code.kill_all();
    }

    /// Pauses the chats that waited long (see `PiBolt::pause_idle_chats`); a chat on Claude Code keeps its process.
    pub fn pause_idle_chats(&self, app: &Arc<App>, now: Instant) {
        self.pi_bolt.pause_idle_chats(app, now);
    }

    fn agent_set(&self) -> std::sync::RwLockReadGuard<'_, AgentSet> {
        self.agents.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The harness of `kind`.
    fn of(&self, kind: HarnessKind) -> &dyn Runs {
        match kind {
            HarnessKind::PiBolt => self.pi_bolt.as_ref(),
            HarnessKind::ClaudeCode => self.claude_code.as_ref(),
        }
    }

    /// The harness the agent `name` runs on; Pi-Bolt for an agent that does not exist, which refuses it.
    fn of_agent(&self, app: &App, name: &str) -> &dyn Runs {
        let agent = resolve_agent(&self.agent_set(), &app.data_dir, name);
        self.of(agent.map_or(HarnessKind::PiBolt, |agent| agent.harness))
    }

    /// The harness that runs the run `run_id`.
    fn of_run(&self, app: &App, run_id: i64) -> &dyn Runs {
        match app.run(run_id) {
            Ok(run) if run.harness == HarnessKind::ClaudeCode.name() => self.claude_code.as_ref(),
            _ => self.pi_bolt.as_ref(),
        }
    }
}

impl CardRuns for Harnesses {
    fn start(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        purpose: (&str, Option<&str>),
        variant: Variant,
    ) -> Result<Run, StartError> {
        self.of_agent(app, agent)
            .start(app, card, agent, purpose, variant)
    }

    fn has_agent(&self, name: &str) -> bool {
        has_agent(&self.agent_set(), name)
    }

    fn runners(&self) -> Vec<crate::harness::remote::RunnerInfo> {
        self.pi_bolt.runners.list()
    }

    fn agents(&self) -> Vec<AgentInfo> {
        agent_list(&self.agent_set())
    }

    fn token_budget(&self, agent: &str) -> Option<i64> {
        self.agent_set().agents.get(agent)?.token_budget
    }

    // A run belongs to exactly one harness; the other does not know the run and says no.
    fn steer(&self, run_id: i64, message: &str) -> bool {
        self.claude_code.steer(run_id, message) || self.pi_bolt.steer(run_id, message)
    }

    fn stop(&self, run_id: i64) -> bool {
        self.claude_code.stop(run_id) || self.pi_bolt.stop(run_id)
    }
}

impl ChatRuns for Harnesses {
    fn tui(&self, app: &Arc<App>, run_id: i64) -> Result<std::process::Command, String> {
        self.of_run(app, run_id).tui(app, run_id)
    }

    fn close_tui(&self, app: &Arc<App>, run_id: i64) {
        self.of_run(app, run_id).close_tui(app, run_id)
    }

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
        self.of_agent(app, agent)
            .start_chat(app, agent, place, message, prompt, origin, settings)
    }

    fn change_chat(&self, app: &Arc<App>, run_id: i64) -> Result<(), String> {
        self.of_run(app, run_id).change_chat(app, run_id)
    }

    // A run belongs to exactly one harness; the other does not know the chat and says no.
    fn say(&self, run_id: i64, message: &str) -> bool {
        self.claude_code.say(run_id, message) || self.pi_bolt.say(run_id, message)
    }

    fn wake(&self, app: &Arc<App>, run_id: i64, prompt: &str) -> Result<(), String> {
        self.of_run(app, run_id).wake(app, run_id, prompt)
    }

    fn end(&self, run_id: i64) -> bool {
        self.claude_code.end(run_id) || self.pi_bolt.end(run_id)
    }
}

impl ImproveRuns for Harnesses {
    fn start_retro(
        &self,
        app: &Arc<App>,
        clone: &Path,
        base: &str,
        prompt: &str,
        files: &BTreeMap<String, String>,
    ) -> Result<Run, StartError> {
        self.of_agent(app, RETRO_AGENT)
            .start_retro(app, clone, base, prompt, files)
    }

    fn start_research(
        &self,
        app: &Arc<App>,
        repository: &Repository,
        base: &str,
        prompt: &str,
    ) -> Result<Run, StartError> {
        self.of_agent(app, AUTORESEARCHER_AGENT)
            .start_research(app, repository, base, prompt)
    }

    fn start_trial(
        &self,
        app: &Arc<App>,
        card: &Card,
        agent: &str,
        definition: AgentConfig,
        proposal: i64,
        base: &str,
    ) -> Result<Run, StartError> {
        self.of(definition.harness)
            .start_trial(app, card, agent, definition, proposal, base)
    }
}

#[cfg(test)]
// Some tests are Unix only; on Windows what only they use is unused.
#[cfg_attr(not(unix), allow(unused))]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use moebius_plugin_api::Host;

    use super::*;
    use crate::repos::git;
    use crate::router::{Protocol, Router};
    use crate::store::Store;

    fn wait_until_ended(app: &App, run_id: i64) -> Run {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let run = app.store.run(run_id).unwrap().unwrap();
            if run.ended_at.is_some() {
                return run;
            }
            assert!(Instant::now() < deadline, "the run did not end: {run:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // Unix only: the fake `claude` and the fake Pi are Python scripts started by their `#!` line.
    #[cfg(unix)]
    #[test]
    fn each_run_goes_to_the_harness_of_its_agent() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(origin.join("README.md"), "hello\n").unwrap();
        git(&origin, &["add", "."]).unwrap();
        git(&origin, &["commit", "--quiet", "-m", "first"]).unwrap();
        let store = Arc::new(Store::in_memory(&dir.path().join("blobs")).unwrap());
        let app = Arc::new(App::new(
            store.clone(),
            Arc::new(Host::new(Duration::from_secs(1))),
            &dir.path().join("data"),
        ));
        let repo = app
            .add_repository("demo", origin.to_str().unwrap())
            .unwrap();
        let coder = AgentConfig {
            model: "m".into(),
            protocol: Protocol::Anthropic,
            thinking: None,
            instructions: None,
            skills: vec![],
            extensions: vec![],
            mcp: BTreeMap::new(),
            token_budget: None,
            harness: HarnessKind::PiBolt,
            tool_output: Default::default(),
            engineering_skills: true,
            runner: None,
        };
        let claude = AgentConfig {
            harness: HarnessKind::ClaudeCode,
            ..coder.clone()
        };
        let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let pi = PiBolt::new(
            testdata.join("fake-pi"),
            "/usr/local/bin/moebius".into(),
            "http://127.0.0.1:9".into(),
            Arc::new(Router::new(store, vec![])),
            BTreeMap::from([("Coder".to_string(), coder), ("Claude".to_string(), claude)]),
        );
        let harnesses = Harnesses::new(pi, &testdata.join("fake-claude"));
        let card = app
            .create_issue(repo.id, "Write done.txt", "", "person")
            .unwrap();

        let run = harnesses
            .start(&app, &card, "Claude", ("code", None), Variant::default())
            .unwrap();
        assert_eq!(run.harness, "claude-code");
        assert_eq!(wait_until_ended(&app, run.id).status, "succeeded");
        let card = app
            .create_issue(repo.id, "Write it again", "", "person")
            .unwrap();
        let run = harnesses
            .start(&app, &card, "Coder", ("code", None), Variant::default())
            .unwrap();
        assert_eq!(run.harness, "pi-bolt");
        assert_eq!(wait_until_ended(&app, run.id).status, "succeeded");

        // A running run is stopped by the harness that runs it.
        let card = app
            .create_issue(repo.id, "Please [hang]", "", "person")
            .unwrap();
        let run = harnesses
            .start(&app, &card, "Claude", ("code", None), Variant::default())
            .unwrap();
        assert!(harnesses.stop(run.id));
        assert_eq!(wait_until_ended(&app, run.id).status, "stopped");
        assert!(
            harnesses
                .agents()
                .iter()
                .any(|a| a.name == "Claude" && a.harness == HarnessKind::ClaudeCode)
        );
    }
}
