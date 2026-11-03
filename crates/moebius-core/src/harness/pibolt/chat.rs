//! Chats: a chat's Pi stays up between the person's messages, works in a worktree of its repository, of the harness
//! configuration repository or in an empty directory, and runs on the model and reasoning level the chat was given.
//! A chat that waited [`IDLE`] loses its Pi, and the next message starts it again on the same session.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use super::process::reasons;
use super::{Closing, IDLE, Live, Paused, PiBolt, lock, send, shut_down};
use crate::app::{App, ChatImage, ChatSettings};
use crate::board::StartError;
use crate::harness::run::{RunningRun, chat_agent, chat_run, finish, launch_or_undo};
use crate::store::{Repository, Run};

impl PiBolt {
    /// Ends the Pi of every chat that has waited [`IDLE`] for the person's next message at `now`, with no terminal
    /// interface open. The chat stays `waiting`, and the next message starts its Pi again on the same session (see
    /// `say`); ending or stopping it ends it at once. The server calls this every minute.
    pub fn pause_idle_chats(&self, app: &Arc<App>, now: Instant) {
        let mut paused = lock(&self.paused);
        let idle: Vec<Live> = lock(&self.live)
            .values()
            .filter(|live| live.chat)
            .filter(|live| {
                lock(&live.waiting_since)
                    .is_some_and(|since| now.saturating_duration_since(since) >= IDLE)
            })
            .cloned()
            .collect();
        for live in idle {
            let id = live.run.id;
            if !app.run(id).is_ok_and(|run| run.status == "waiting") || !self.end_waiting_pi(&live)
            {
                continue;
            }
            paused.insert(
                id,
                Paused {
                    run: live.run,
                    follower: None,
                    closing: None,
                    app: app.clone(),
                },
            );
        }
    }

    /// Ends the Pi of a chat that waits, which leaves the chat as it is: false when the chat was ending already.
    pub(super) fn end_waiting_pi(&self, live: &Live) -> bool {
        {
            let mut closing = lock(&live.closing);
            if closing.is_some() {
                return false;
            }
            *closing = Some(Closing::Paused);
        }
        // It is idle, as the chat waits.
        shut_down(live);
        lock(&self.live).remove(&live.run.id);
        true
    }

    /// Starts the Pi of a chat that has none again, on the chat's session, waiting for the next message.
    pub(super) fn relaunch(&self, app: &Arc<App>, run: &RunningRun) -> Result<(), String> {
        let agent = app.run(run.id).map_err(|e| e.to_string())?.agent;
        let config = self.chat_config(app, run.id, &agent)?;
        self.launch(app, None, &config, run.clone(), (None, &[]))
    }

    /// Ends (`closing`) the chat `run_id` if it has no Pi of its own: on a thread of its own at once when it waited
    /// [`IDLE`], or when its terminal interface closes. False when the chat has a Pi, or is no chat.
    pub(super) fn close_paused(
        &self,
        paused: &mut HashMap<i64, Paused>,
        run_id: i64,
        closing: Closing,
    ) -> bool {
        let Some(chat) = paused.get_mut(&run_id) else {
            return false;
        };
        if chat.follower.is_some() {
            chat.closing = Some(closing);
            return true;
        }
        let chat = paused.remove(&run_id).expect("the chat is paused");
        let (router, lives) = (self.router.clone(), self.live.clone());
        let stopped = closing == Closing::Stopped;
        std::thread::spawn(move || finish(&chat.app, chat.run, &router, &lives, Ok(stopped)));
        true
    }

    /// Starts a chat (see `Runs::start_chat`): its worktree, its token, what the person said and the settings it was
    /// given (see [`chat_run`]), then its Pi.
    pub(super) fn open_chat(
        &self,
        app: &Arc<App>,
        agent: &str,
        place: (Option<&Repository>, bool),
        (message, images, prompt): (Option<&str>, &[ChatImage], Option<&str>),
        origin: &str,
        settings: &ChatSettings,
    ) -> Result<Run, StartError> {
        let config = chat_agent(&self.agent_set(), app, agent, settings)
            .map_err(StartError::UnknownAgent)?;
        let running = chat_run(
            app,
            &self.router,
            (agent, &config),
            place,
            (message, images),
            origin,
            settings,
        )?;
        launch_or_undo(app, &self.router, running, |run| {
            self.launch(app, None, &config, run, (prompt, images))
        })
    }

    /// Tells the chat's Pi the reasoning level it was given with `set_thinking_level`; a chat given another model, or a
    /// level its Pi's `models.json` cannot reason with, gets a new Pi on the same session, waiting for the next
    /// message. A chat without a Pi meanwhile (paused, or in its terminal) is started with them the next time.
    pub(super) fn apply_chat_settings(&self, app: &Arc<App>, run_id: i64) -> Result<(), String> {
        // Held throughout, so that the chat does not move to or from its terminal interface or pause meanwhile.
        let paused = lock(&self.paused);
        if paused.contains_key(&run_id) {
            return Ok(());
        }
        let Some(live) = self.live_chat(run_id) else {
            return Err(format!("chat {run_id} has ended"));
        };
        let stored = app.run(run_id).map_err(|e| e.to_string())?;
        let config = self.chat_config(app, run_id, &stored.agent)?;
        let (was, now) = (&live.agent, &config);
        if was.model == now.model && was.protocol == now.protocol && reasons(was) == reasons(now) {
            if reasons(now) && was.thinking != now.thinking {
                let level = now.thinking.as_deref().unwrap_or("off");
                let told = json!({ "id": "moebius-settings", "type": "set_thinking_level", "level": level });
                if !send(&live.stdin, &told) {
                    return Err(format!("chat {run_id} has ended"));
                }
                lock(&self.live)
                    .entry(run_id)
                    .and_modify(|live| live.agent = config);
            }
            return Ok(());
        }
        if !self.end_waiting_pi(&live) {
            return Err(format!("chat {run_id} has ended"));
        }
        if let Err(e) = self.relaunch(app, &live.run) {
            let why = format!("cannot start the chat's pi-bolt again with its new settings: {e}");
            finish(app, live.run, &self.router, &self.live, Err(why.clone()));
            return Err(why);
        }
        Ok(())
    }

    /// Tells the chat's Pi the person's `message`, starting it again first when the chat waited [`IDLE`]; false when
    /// the chat has ended.
    pub(super) fn tell_chat(&self, run_id: i64, message: &str, images: &[ChatImage]) -> bool {
        // Held throughout, so that the chat is not paused while it is told the message.
        let mut paused = lock(&self.paused);
        // A chat that waited long gets its Pi again, on the same session.
        if paused.get(&run_id).is_some_and(Paused::is_idle) {
            let Paused { run, app, .. } = paused.remove(&run_id).expect("the chat is paused");
            if let Err(e) = self.relaunch(&app, &run) {
                let why = format!("cannot start the chat's pi-bolt again: {e}");
                finish(&app, run, &self.router, &self.live, Err(why));
                return false;
            }
            // Started, it waits; the message makes it work.
            let _ = app.store.set_chat_working(run_id);
        }
        let Some(live) = self.live_chat(run_id) else {
            return false;
        };
        *lock(&live.waiting_since) = None;
        // Pi starts a run for the prompt, or queues it when the agent is working, so the answer does not depend on
        // whether the agent has just settled.
        let mut said =
            json!({ "type": "prompt", "message": message, "streamingBehavior": "followUp" });
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
        send(&live.stdin, &said)
    }

    /// Ends the chat: its Pi is asked to shut down, and the chat ends once it has. False when the chat has ended.
    pub(super) fn end_chat(&self, run_id: i64) -> bool {
        // Held throughout, so that the chat does not move to or from its terminal interface meanwhile.
        let mut paused = lock(&self.paused);
        if self.close_paused(&mut paused, run_id, Closing::Ended) {
            return true;
        }
        let Some(live) = self.live_chat(run_id) else {
            return false;
        };
        *lock(&live.closing) = Some(Closing::Ended);
        // Closing stdin asks Pi to shut down.
        lock(&live.stdin).take().is_some()
    }
}

// Unix only: the fake Pi is a Python script started by its `#!` line.
#[cfg(all(test, unix))]
mod tests {

    use serde_json::{Value, json};

    use super::*;
    use crate::app::AppError;
    use crate::board::ChatRuns;
    use crate::decide::Actor;
    use crate::harness::pibolt::testing::{
        argv_and_stdin, flag, setup, wait_for, wait_for_run, wait_until_ended,
    };
    use crate::router::Protocol;

    #[test]
    fn a_chat_idle_for_long_loses_its_pi_and_the_next_message_starts_it_again() {
        let (_dir, app, pi, repo) = setup();
        let repository = app.repository(repo).unwrap();
        let run = pi
            .start_chat(
                &app,
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
        wait_for_run(&app, run.id, "the agent answers", waiting);
        let agent_dir = app.data_dir.join(format!("runs/{}/agent", run.id));
        let pid = std::fs::read_to_string(agent_dir.join("pid.txt")).unwrap();

        // Before IDLE has passed the chat keeps its Pi.
        pi.pause_idle_chats(&app, Instant::now());
        assert!(lock(&pi.live).contains_key(&run.id));
        pi.pause_idle_chats(&app, Instant::now() + IDLE);
        assert!(!lock(&pi.live).contains_key(&run.id));
        assert!(!alive(&pid));
        assert_eq!(app.run(run.id).unwrap().status, "waiting");

        // The next message starts Pi again on the same session, and it answers.
        app.say_in_chat(&pi, run.id, "Again", &[], None).unwrap();
        let answered = |run: &Run| {
            run.status == "waiting"
                && app
                    .store
                    .events(run.id)
                    .unwrap()
                    .iter()
                    .filter(|e| e.kind == "agent_settled")
                    .count()
                    == 2
        };
        wait_for_run(&app, run.id, "the agent answers again", answered);
        let argv = std::fs::read_to_string(agent_dir.join("argv.json")).unwrap();
        assert!(argv.contains("--continue"), "{argv}");
        assert_eq!(
            std::fs::read_to_string(agent_dir.join("prompt.txt")).unwrap(),
            "Again"
        );

        // A chat that is paused ends at once when it is ended.
        pi.pause_idle_chats(&app, Instant::now() + IDLE);
        assert!(lock(&pi.paused).contains_key(&run.id));
        app.end_chat(&pi, run.id).unwrap();
        assert_eq!(wait_until_ended(&app, run.id).status, "succeeded");
        assert!(!pi.say(run.id, "Hello?", &[]));
    }

    /// Whether the process `pid` still runs: it exists and has not exited (a zombie has, it only waits to be reaped).
    fn alive(pid: &str) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, rest)| !rest.starts_with('Z'))
        })
    }

    #[test]
    fn a_chat_runs_on_the_model_and_reasoning_it_was_given_and_keeps_them() {
        let (_dir, app, pi, _repo) = setup();
        let chosen = (Some("big-model"), Some("high"), &[][..]);
        let run = app
            .start_chat(
                &pi,
                Some("Coder"),
                None,
                false,
                None,
                &[],
                None,
                None,
                None,
                chosen,
            )
            .unwrap();
        // Without a message the chat's Pi starts and the chat waits for the first one.
        assert_eq!(run.status, "waiting");
        let (argv, stdin) = argv_and_stdin(&app, run.id);
        assert_eq!(flag(&argv, "--model"), Some("big-model"));
        assert_eq!(flag(&argv, "--thinking"), Some("high"));
        assert!(stdin.is_empty(), "{stdin:?}");
        let models: Value = serde_json::from_slice(
            &std::fs::read(
                app.data_dir
                    .join(format!("runs/{}/agent/models.json", run.id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            models["providers"]["moebius"]["models"],
            json!([{ "id": "big-model", "reasoning": true }])
        );
        let listed = &app.chats(false).unwrap()[0];
        assert_eq!(listed.model.as_deref(), Some("big-model"));
        assert_eq!(listed.thinking.as_deref(), Some("high"));
        assert_eq!(listed.title, "");

        // Another reasoning level is told to the waiting Pi.
        let pid_file = app.data_dir.join(format!("runs/{}/agent/pid.txt", run.id));
        wait_for("Pi writes its pid", || {
            std::fs::read_to_string(&pid_file).is_ok_and(|pid| !pid.trim().is_empty())
        });
        let pid = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .to_string();
        app.change_chat_settings(&pi, run.id, (None, Some("low"), &[]))
            .unwrap();
        wait_for("Pi is told the level", || {
            !argv_and_stdin(&app, run.id).1.is_empty()
        });
        let (_, stdin) = argv_and_stdin(&app, run.id);
        assert_eq!(
            stdin,
            [json!({ "id": "moebius-settings", "type": "set_thinking_level", "level": "low" })]
        );
        assert!(alive(&pid));

        // Another model needs a new Pi on the same session, started with it.
        app.change_chat_settings(&pi, run.id, (Some("other-model"), None, &[]))
            .unwrap();
        assert_eq!(app.run(run.id).unwrap().status, "waiting");
        wait_for("the first Pi ends", || !alive(&pid));
        wait_for("Pi starts again", || {
            flag(&argv_and_stdin(&app, run.id).0, "--model") == Some("other-model")
        });
        let (argv, _) = argv_and_stdin(&app, run.id);
        assert_eq!(flag(&argv, "--model"), Some("other-model"));
        assert_eq!(flag(&argv, "--thinking"), Some("low"));
        assert!(argv.contains(&"--continue".to_string()), "{argv:?}");
        let kinds: Vec<String> = app
            .store
            .events(run.id)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "settings_changed")
            .map(|e| e.data.to_string())
            .collect();
        assert_eq!(kinds.len(), 3, "{kinds:?}");

        // A Pi started again after the chat waited long keeps them.
        pi.pause_idle_chats(&app, Instant::now() + IDLE);
        assert!(lock(&pi.paused).contains_key(&run.id));
        app.change_chat_settings(&pi, run.id, (None, Some("medium"), &[]))
            .unwrap();
        app.say_in_chat(&pi, run.id, "Hello", &[], None).unwrap();
        wait_for_run(&app, run.id, "the agent answers", |run| {
            run.status == "waiting" && app.store.answered_seq(run.id).unwrap() > 0
        });
        let (argv, _) = argv_and_stdin(&app, run.id);
        assert_eq!(flag(&argv, "--model"), Some("other-model"));
        assert_eq!(flag(&argv, "--thinking"), Some("medium"));
        assert_eq!(app.chats(false).unwrap()[0].title, "Hello");
        app.end_chat(&pi, run.id).unwrap();
        wait_until_ended(&app, run.id);
    }

    #[test]
    fn a_working_chat_keeps_its_settings_and_a_level_pi_does_not_take_is_refused() {
        let (_dir, app, pi, _repo) = setup();
        let run = app
            .start_chat(
                &pi,
                Some("Coder"),
                None,
                false,
                Some("[hang]"),
                &[],
                None,
                None,
                None,
                (None, None, &[]),
            )
            .unwrap();
        assert_eq!(run.status, "running");
        let refused = app.change_chat_settings(&pi, run.id, (None, Some("high"), &[]));
        assert!(matches!(refused, Err(AppError::Conflict(_))), "{refused:?}");
        let refused = app.change_chat_settings(&pi, run.id, (None, Some("huge"), &[]));
        assert!(matches!(refused, Err(AppError::Invalid(_))), "{refused:?}");
        assert!(app.chat_settings(run.id).unwrap() == ChatSettings::default());
        // A chat started without settings runs on its agent's.
        let (argv, _) = argv_and_stdin(&app, run.id);
        assert_eq!(flag(&argv, "--model"), Some("qwen-local"));
        app.stop_run(&Actor::Person, &pi, run.id, "").unwrap();
        wait_until_ended(&app, run.id);
    }

    #[test]
    fn a_chats_model_takes_the_protocol_the_router_serves_it_in() {
        let (_dir, app, pi, _repo) = setup();
        // The Coder speaks OpenAI's protocol; a model served only in Anthropic's is spoken to in that.
        let chosen = (Some("claude-like"), None, &[Protocol::Anthropic][..]);
        let run = app
            .start_chat(
                &pi,
                Some("Coder"),
                None,
                false,
                None,
                &[],
                None,
                None,
                None,
                chosen,
            )
            .unwrap();
        assert_eq!(
            app.chat_settings(run.id).unwrap().protocol,
            Some(Protocol::Anthropic)
        );
        let models: Value = serde_json::from_slice(
            &std::fs::read(
                app.data_dir
                    .join(format!("runs/{}/agent/models.json", run.id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(models["providers"]["moebius"]["api"], "anthropic-messages");
        // One served in both keeps the agent's.
        let both = &[Protocol::Anthropic, Protocol::OpenAi][..];
        app.change_chat_settings(&pi, run.id, (Some("shared"), None, both))
            .unwrap();
        assert_eq!(app.chat_settings(run.id).unwrap().protocol, None);
        app.end_chat(&pi, run.id).unwrap();
        wait_until_ended(&app, run.id);
    }
}
