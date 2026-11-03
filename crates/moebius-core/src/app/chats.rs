//! Chats: a person starting one with an agent, in a repository, the harness configuration repository or nowhere,
//! writing to it (a chat whose agent went away is taken up again by the next message), changing its model and
//! reasoning, ending it and continuing it later, and the list of chats.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{App, AppError, Result};
use crate::blobs::redact;
use crate::board::Runs;
use crate::bus::Event;
use crate::config::AgentConfig;
use crate::harness::terminal::tui_key;
use crate::repos::git;
use crate::router::Protocol;
use crate::store::Run;

/// How many characters of a chat's first message its title keeps.
const TITLE_LENGTH: usize = 80;

/// A chat as the list of chats shows it. Its title is the first line of its first message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    pub agent: String,
    pub status: String,
    pub repository_id: Option<i64>,
    /// Whether the chat works in the harness configuration repository.
    pub harness_repository: bool,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub title: String,
    /// The `seq` of the chat's last answer or error, which the person has to read.
    pub answered: i64,
    /// How far the person has read the chat (`POST /v1/chats/{id}/seen`); unread while below `answered`.
    pub seen: i64,
    /// How many files the chat's worktree changes against the default branch, committed or not: what
    /// `POST /v1/chats/{id}/offer` would offer. 0 for an ended chat or one without a repository.
    #[serde(default)]
    pub changed_files: usize,
    /// Whether the chat waits for an answer to a question: it waits, and the agent's last answer ends in one.
    #[serde(default)]
    pub asks: bool,
    /// Where the chat was started: `chat`, `assistant` or `automation` (see `Run::origin`).
    #[serde(default = "chat_origin")]
    pub origin: String,
    /// The model the chat was given in place of its agent's; none while it runs on its agent's.
    #[serde(default)]
    pub model: Option<String>,
    /// The reasoning level the chat was given in place of its agent's; none while it runs with its agent's.
    #[serde(default)]
    pub thinking: Option<String>,
}

/// Whether an answer ends in a question to the person: its last line, without the marks of emphasis or a closing
/// bracket or quote, ends in `?`.
fn asks(answer: &str) -> bool {
    answer
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .is_some_and(|line| {
            line.trim_end_matches(['*', '_', '`', ')', '"', '\'', '»', '“', '”'])
                .ends_with(['?', '？'])
        })
}

/// The reasoning levels a chat can be given, as Pi-Bolt takes them (`--thinking`, `set_thinking_level`).
pub const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// What a chat runs with in place of its agent's: a model, with the protocol the router serves it in, and a reasoning
/// level; each is the agent's while none. A chat keeps them as `settings_changed` events, the latest of which counts,
/// so that its Pi is started with them again after it was paused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSettings {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub protocol: Option<Protocol>,
    #[serde(default)]
    pub thinking: Option<String>,
}

impl ChatSettings {
    /// `agent` as a chat with these settings runs it.
    pub fn apply(&self, agent: &AgentConfig) -> AgentConfig {
        let mut agent = agent.clone();
        if let Some(model) = &self.model {
            agent.model = model.clone();
            agent.protocol = self.protocol.unwrap_or(agent.protocol);
        }
        if let Some(thinking) = &self.thinking {
            agent.thinking = Some(thinking.clone());
        }
        agent
    }
}

/// The protocol a chat speaks to `model` when its agent speaks `agent`: the agent's, unless the router's upstreams serve
/// the model (`served`, the protocols they list it under) only in others, then the first of those.
fn chat_protocol(agent: Option<Protocol>, served: &[Protocol]) -> Option<Protocol> {
    match agent {
        Some(agent) if served.is_empty() || served.contains(&agent) => None,
        _ => served.first().copied(),
    }
}

/// The protocol the agent `name` speaks, if `runs` knows it and it has a model.
fn agent_protocol(runs: &dyn Runs, name: &str) -> Option<Protocol> {
    runs.agents()
        .into_iter()
        .find(|agent| agent.name == name && !agent.model.is_empty())
        .map(|agent| agent.protocol)
}

/// Refuses a blank model and a reasoning level Pi-Bolt does not take.
fn check_settings(model: Option<&str>, thinking: Option<&str>) -> Result<()> {
    if model.is_some_and(|model| model.trim().is_empty()) {
        return Err(AppError::Invalid("a chat's model cannot be blank".into()));
    }
    if let Some(level) = thinking.filter(|level| !THINKING_LEVELS.contains(level)) {
        return Err(AppError::Invalid(format!(
            "a chat's reasoning is one of {}, not {level}",
            THINKING_LEVELS.join(", ")
        )));
    }
    Ok(())
}

/// The origin of a chat started on the Chat page or by a client.
pub const CHAT_ORIGIN: &str = "chat";

/// The origin of a chat started in the desktop app's floating window, which the list of chats leaves out.
pub const ASSISTANT_ORIGIN: &str = "assistant";

fn chat_origin() -> String {
    CHAT_ORIGIN.into()
}

/// The most characters of an ended chat's conversation that the chat continuing it is given.
const CONVERSATION_LIMIT: usize = 20_000;

/// How many characters of where a person is in the app a chat's agent is told with a message, at most.
const CONTEXT_LIMIT: usize = 4_000;

/// What a chat's agent is told before the person's message when the chat takes up a conversation that had ended: what
/// was said in it, which the person already sees.
fn continues_prompt(earlier: &str, message: &str) -> String {
    format!(
        "This chat continues an earlier one, which ended. What was said in it:\n\n{earlier}\n\n\
         The person now writes:\n\n{message}"
    )
}

/// The prompt a chat's agent answers: `message`, after `context`, where the person who wrote it is in the app, with
/// its secrets redacted and cut to [`CONTEXT_LIMIT`] characters. Without a context, or with a blank one, the message.
fn with_context(context: Option<&str>, message: &str) -> String {
    let Some(context) = context.map(str::trim).filter(|c| !c.is_empty()) else {
        return message.to_string();
    };
    let context: String = redact(context).chars().take(CONTEXT_LIMIT).collect();
    format!(
        "[Where the person is in the Moebius app. Use it to understand what they mean; do not repeat it.]\n\
         {context}\n\n{message}"
    )
}

impl App {
    /// Makes the worktree of the chat `run_id` in the harness configuration repository, on a new branch of its default
    /// branch, and records that the chat works there. Returns the worktree and the clone it belongs to.
    pub(crate) fn start_harness_chat(
        &self,
        run_id: i64,
    ) -> Result<(crate::repos::Worktree, PathBuf)> {
        let repo = self.harness_repo()?;
        self.store.set_run_harness_repository(run_id)?;
        Ok((
            repo.add_chat_worktree(self, run_id)?,
            repo.path().to_path_buf(),
        ))
    }

    /// A person starts a chat with `agent`, in a branch of the repository `repository_id`, of the harness configuration
    /// repository when `harness` is set, or, without either, in an empty directory. The agent answers `message`, told
    /// first where the person is in the app when `context` is given (see `with_context`); the chat shows only the
    /// message. The answer arrives as the run's events. Without a message the chat's agent starts and the chat waits
    /// for the first one, so that its terminal can be opened at once. The chat is stored with `origin`, `chat` when
    /// none is given (see `Run::origin`), and runs on `model` and with the reasoning level `thinking` in place of its
    /// agent's when they are given (see `ChatSettings`); `served` are the protocols the router's upstreams list the
    /// model under.
    #[allow(clippy::too_many_arguments)]
    pub fn start_chat(
        self: &Arc<Self>,
        runs: &dyn Runs,
        agent: Option<&str>,
        repository_id: Option<i64>,
        harness: bool,
        message: Option<&str>,
        context: Option<&str>,
        continues: Option<i64>,
        origin: Option<&str>,
        (model, thinking, served): (Option<&str>, Option<&str>, &[Protocol]),
    ) -> Result<Run> {
        let origin = origin.unwrap_or(CHAT_ORIGIN);
        let automation = super::automations::AUTOMATION_ORIGIN;
        if ![CHAT_ORIGIN, ASSISTANT_ORIGIN, automation].contains(&origin) {
            return Err(AppError::Invalid(format!(
                "a chat's origin is {CHAT_ORIGIN}, {ASSISTANT_ORIGIN} or {automation}, not {origin}"
            )));
        }
        if harness && repository_id.is_some() {
            return Err(AppError::Invalid(
                "a chat works in a repository or in the harness configuration, not both".into(),
            ));
        }
        // A chat that continues an ended one keeps its agent, place, model and reasoning unless others are named.
        let earlier = continues.map(|id| self.ended_chat(id)).transpose()?;
        let agent = agent
            .or(earlier.as_ref().map(|run| run.agent.as_str()))
            .unwrap_or(crate::harness::pibolt::MOEBIUS_AGENT);
        let harness = harness
            || (repository_id.is_none()
                && earlier.as_ref().is_some_and(|run| run.harness_repository));
        let repository_id = repository_id.or(earlier.as_ref().and_then(|run| run.repository_id));
        if message.is_some_and(|message| message.trim().is_empty()) {
            return Err(AppError::Invalid("a chat's message cannot be blank".into()));
        }
        if message.is_none() && earlier.is_some() {
            return Err(AppError::Invalid(
                "a chat that continues an ended one needs a message".into(),
            ));
        }
        check_settings(model, thinking)?;
        if !runs.has_agent(agent) {
            return Err(AppError::Invalid(format!("no agent named {agent}")));
        }
        if harness {
            self.harness_repo()?;
        }
        if agent == crate::harness::pibolt::GRILLING_AGENT && repository_id.is_none() {
            return Err(AppError::Invalid(
                "a grilling needs a repository, whose issues it ends in".into(),
            ));
        }
        let mut settings = match &earlier {
            Some(run) => self.chat_settings(run.id)?,
            None => ChatSettings::default(),
        };
        if let Some(model) = model {
            settings.model = Some(model.trim().to_string());
            settings.protocol = chat_protocol(agent_protocol(runs, agent), served);
        }
        if let Some(thinking) = thinking {
            settings.thinking = Some(thinking.to_string());
        }
        let repository = repository_id.map(|id| self.repository(id)).transpose()?;
        let conversation = earlier
            .as_ref()
            .map(|run| self.conversation_of(run.id))
            .transpose()?;
        // The agent is also told what was said before, when the chat continues an ended one.
        let prompt = message.map(|message| {
            let told = match &conversation {
                Some(earlier) => continues_prompt(earlier, message),
                None => message.to_string(),
            };
            with_context(context, &told)
        });
        let place = (repository.as_ref(), harness);
        runs.start_chat(
            self,
            agent,
            place,
            message,
            prompt.as_deref(),
            origin,
            &settings,
        )
        .map_err(|e| AppError::Invalid(e.to_string()))
    }

    /// What the chat `run_id` runs with in place of its agent's: what its latest `settings_changed` event holds.
    pub fn chat_settings(&self, run_id: i64) -> Result<ChatSettings> {
        Ok(self
            .store
            .event_of_kind(run_id, "settings_changed", true)?
            .and_then(|event| serde_json::from_value(event.data).ok())
            .unwrap_or_default())
    }

    /// A person changes the model or the reasoning level of the chat `run_id` (see `ChatSettings`); what is not given
    /// stays. The chat must be waiting: while its agent works the answer is a conflict. The change is kept as a
    /// `settings_changed` event and told to the chat's Pi (see [`Runs::change_chat`]), so that the next message is
    /// answered with it. `served` are the protocols the router's upstreams list `model` under.
    pub fn change_chat_settings(
        self: &Arc<Self>,
        runs: &dyn Runs,
        run_id: i64,
        (model, thinking, served): (Option<&str>, Option<&str>, &[Protocol]),
    ) -> Result<Run> {
        let run = self.running_chat(run_id)?;
        check_settings(model, thinking)?;
        let _turn = self.chat_turns.lock().unwrap_or_else(|e| e.into_inner());
        let run = self.run(run.id)?;
        if run.status != "waiting" {
            return Err(AppError::Conflict(format!(
                "chat {run_id} is {}: its model and reasoning change once the agent has answered",
                run.status
            )));
        }
        let mut settings = self.chat_settings(run_id)?;
        if let Some(model) = model {
            settings.model = Some(model.trim().to_string());
            settings.protocol = chat_protocol(agent_protocol(runs, &run.agent), served);
        }
        if let Some(thinking) = thinking {
            settings.thinking = Some(thinking.to_string());
        }
        self.record_run_event(run_id, "settings_changed", &json!(settings))?;
        runs.change_chat(self, run_id).map_err(AppError::Invalid)?;
        self.run(run_id)
    }

    /// The chat `run_id`, which must have ended to be continued.
    fn ended_chat(&self, run_id: i64) -> Result<Run> {
        let run = self.chat(run_id)?;
        if run.ended_at.is_none() {
            return Err(AppError::Invalid(format!(
                "chat {run_id} has not ended; write to it instead"
            )));
        }
        Ok(run)
    }

    /// What was said in a chat, oldest first: the person's messages and the agent's answers, at most the last
    /// [`CONVERSATION_LIMIT`] characters of it.
    fn conversation_of(&self, run_id: i64) -> Result<String> {
        let lines: Vec<String> = self
            .store
            .events(run_id)?
            .iter()
            .filter_map(|event| match event.kind.as_str() {
                "said" => Some(format!("Person: {}", event.data["message"].as_str()?)),
                "agent_end" => Some(format!(
                    "Agent: {}",
                    crate::harness::pibolt::final_answer(&event.data)?
                )),
                _ => None,
            })
            .collect();
        let text = lines.join("\n\n");
        let start = text
            .char_indices()
            .rev()
            .nth(CONVERSATION_LIMIT.saturating_sub(1))
            .map_or(0, |(i, _)| i);
        Ok(text[start..].to_string())
    }

    /// A person writes to a chat. While its agent is up, the chat keeps the message as a `said` event, and the agent
    /// answers it when it is done with what it is doing, told first where the person is when `context` is given. A chat
    /// that ended because its agent went away is taken up again with what was said in it (see [`App::wake_chat`]), so
    /// that the person carries on in the chat they are reading instead of starting a new one. A chat that ended any
    /// other way is ended for good: it is continued by a new chat (see [`App::start_chat`]).
    pub fn say_in_chat(
        self: &Arc<Self>,
        runs: &dyn Runs,
        run_id: i64,
        message: &str,
        context: Option<&str>,
    ) -> Result<Run> {
        let run = self.chat(run_id)?;
        if message.trim().is_empty() {
            return Err(AppError::Invalid("a message cannot be blank".into()));
        }
        if self.terminals.is_open(tui_key(run_id)) {
            return Err(AppError::Conflict(format!(
                "chat {run_id} is in its terminal: close it to write to the chat"
            )));
        }
        if run.ended_at.is_some() {
            return self.wake_chat(runs, &run, message, context);
        }
        let _turn = self.chat_turns.lock().unwrap_or_else(|e| e.into_inner());
        // Stored first, so that it comes before the agent's records of its answer.
        self.record_run_event(run_id, "said", &json!({ "message": redact(message) }))?;
        // The chat works from now on, before its agent says so, so that its worktree is not offered meanwhile.
        self.store.set_chat_working(run_id)?;
        if !runs.say(run_id, &with_context(context, message)) {
            return Err(AppError::Invalid(format!("chat {run_id} has ended")));
        }
        Ok(run)
    }

    /// A person writes to the ended chat `run`, which ended because its agent went away: the agent starts again in the
    /// chat's own place and on its own branch, told what was said in the chat before, and the chat works from now on.
    /// What the person wrote is the message it answers, told first where they are when `context` is given. When the
    /// agent cannot start again the chat is left as it was, and why it cannot is the error.
    fn wake_chat(
        self: &Arc<Self>,
        runs: &dyn Runs,
        run: &Run,
        message: &str,
        context: Option<&str>,
    ) -> Result<Run> {
        let run_id = run.id;
        if run.status != "stopped" {
            return Err(AppError::Invalid(format!("chat {run_id} has ended")));
        }
        if run.agent == crate::harness::pibolt::GRILLING_AGENT {
            return Err(AppError::Invalid(format!(
                "chat {run_id} is a grilling, whose issues are already read: start a new grilling"
            )));
        }
        let _turn = self.chat_turns.lock().unwrap_or_else(|e| e.into_inner());
        let told = continues_prompt(&self.conversation_of(run_id)?, message);
        // Stored first, so that it comes before the agent's records of its answer.
        self.record_run_event(run_id, "said", &json!({ "message": redact(message) }))?;
        // The chat works from now on and is no longer ended, before its agent says so.
        self.store.reopen_chat(run_id)?;
        if let Err(e) = runs.wake(self, run_id, &with_context(context, &told)) {
            // As it was: ended because its agent went away, its worktree taken away again with it.
            let _ = self.store.set_run_status(run_id, "stopped");
            return Err(AppError::Invalid(format!(
                "chat {run_id} cannot start again: {e}"
            )));
        }
        self.run(run_id)
    }

    /// A person ends a chat that has not ended. It ends as `succeeded` once its agent has shut down and its work is
    /// committed and, if there is any, offered as a change (see `offer_chat_change`).
    pub fn end_chat(&self, runs: &dyn Runs, run_id: i64) -> Result<Run> {
        let run = self.running_chat(run_id)?;
        if !runs.end(run_id) {
            return Err(AppError::Invalid(format!("chat {run_id} has ended")));
        }
        // A chat in its terminal interface ends once the interface has closed.
        self.terminals.close_tui(run_id);
        Ok(run)
    }

    /// The chat `run_id`, if it exists and has not ended.
    pub(super) fn running_chat(&self, run_id: i64) -> Result<Run> {
        let run = self.chat(run_id)?;
        if run.ended_at.is_some() {
            return Err(AppError::Invalid(format!("chat {run_id} has ended")));
        }
        Ok(run)
    }

    /// The run `run_id`, which must be a chat.
    fn chat(&self, run_id: i64) -> Result<Run> {
        let run = self.run(run_id)?;
        if run.purpose != "chat" {
            return Err(AppError::Invalid(format!("run {run_id} is not a chat")));
        }
        Ok(run)
    }

    /// The chats, newest first; those of the desktop app's floating window only when `all` is set.
    pub fn chats(&self, all: bool) -> Result<Vec<Chat>> {
        self.store
            .chats()?
            .into_iter()
            .filter(|run| all || run.origin != ASSISTANT_ORIGIN)
            .map(|run| self.chat_view(&run))
            .collect()
    }

    /// The chat `run` as the list of chats shows it.
    fn chat_view(&self, run: &Run) -> Result<Chat> {
        let settings = self.chat_settings(run.id)?;
        Ok(Chat {
            id: run.id,
            agent: run.agent.clone(),
            status: run.status.clone(),
            repository_id: run.repository_id,
            harness_repository: run.harness_repository,
            started_at: run.started_at,
            ended_at: run.ended_at,
            title: self.chat_title(run)?,
            answered: self.store.answered_seq(run.id)?,
            seen: self.store.seen_seq(run.id)?,
            changed_files: self.changed_files(run),
            asks: run.status == "waiting" && asks(run.summary.as_deref().unwrap_or_default()),
            origin: run.origin.clone(),
            model: settings.model,
            thinking: settings.thinking,
        })
    }

    /// Names the chat `run_id` `title`, in place of the title taken from its first message, and returns it as the list
    /// of chats shows it. An empty name gives that title back.
    pub fn rename_chat(&self, run_id: i64, title: &str) -> Result<Chat> {
        let run = self.chat(run_id)?;
        let named: String = title.trim().chars().take(TITLE_LENGTH).collect();
        let title = if named.is_empty() { None } else { Some(named) };
        self.store.set_run_title(run_id, title.as_deref())?;
        let chat = self.chat_view(&Run { title, ..run })?;
        self.bus.publish(Event::ChatRenamed {
            run: run_id,
            title: chat.title.clone(),
        });
        Ok(chat)
    }

    /// A person has read the chat `run_id` up to the event `seq`.
    pub fn mark_chat_seen(&self, run_id: i64, seq: i64) -> Result<()> {
        self.chat(run_id)?;
        Ok(self.store.set_seen_seq(run_id, seq)?)
    }

    /// The title of a chat: the name a person gave it, or the first line of its first message.
    pub(super) fn chat_title(&self, run: &Run) -> Result<String> {
        if let Some(title) = run.title.as_deref() {
            return Ok(title.to_owned());
        }
        let run_id = run.id;
        let first = self.store.event_of_kind(run_id, "said", false)?;
        Ok(first
            .as_ref()
            .and_then(|event| event.data["message"].as_str())
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(TITLE_LENGTH)
            .collect())
    }

    /// How many files the worktree of the chat `run` changes against its default branch, committed or not; 0 for an
    /// ended chat, one without a repository, or when git cannot tell.
    fn changed_files(&self, run: &Run) -> usize {
        let Some(worktree) = run.worktree.as_deref().filter(|_| run.ended_at.is_none()) else {
            return 0;
        };
        let base = if run.harness_repository {
            self.harness_repo()
                .ok()
                .and_then(|repo| self.repos.default_branch(repo.path()).ok())
        } else {
            run.repository_id
                .and_then(|id| self.repository(id).ok())
                .map(|repository| repository.base_ref())
        };
        let Some(base) = base else {
            return 0;
        };
        let worktree = Path::new(worktree);
        let committed = format!("{base}...HEAD");
        let lists = [
            git(worktree, &["diff", "--name-only", &committed]),
            git(worktree, &["diff", "--name-only", "HEAD"]),
            git(worktree, &["ls-files", "--others", "--exclude-standard"]),
        ];
        let mut files = BTreeSet::new();
        for list in lists {
            let Ok(list) = list else {
                return 0;
            };
            files.extend(list.lines().map(str::to_string));
        }
        files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::asks;

    #[test]
    fn an_answer_asks_when_its_last_line_ends_in_a_question() {
        assert!(asks("I found two ways.\n\nShould I take the **first**?"));
        assert!(asks("Which repository do you mean?\n\n"));
        assert!(asks("Soll ich weitermachen (ja/nein)?"));
        assert!(!asks(
            "Done. Is that what you wanted? I also ran the tests."
        ));
        assert!(!asks("The tests pass."));
        assert!(!asks(""));
    }
}
