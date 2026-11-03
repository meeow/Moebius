//! Chats: a person starting one with an agent, in a repository, the harness configuration repository or nowhere,
//! writing to it with words and the pictures its model can look at (a chat whose agent went away is taken up
//! again by the next message), changing its model and reasoning, ending it and continuing it later, and the list
//! of chats.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{App, AppError, Result};
use crate::blobs::redact;
use crate::board::Runs;
use crate::bus::Event;
use crate::config::{AgentConfig, THINKING_LEVELS};
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

/// An image the person attaches to a chat message: what they called it, which kind of image it is, and its bytes.
/// The bytes go to the chat's agent along with the message; a copy is kept under the chat, so the chat can show what
/// was sent even after its agent is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatImage {
    /// What the person's browser or clipboard called it, which is what the chat shows them; none is fine.
    pub name: Option<String>,
    /// One of the kinds of [`IMAGE_KINDS`].
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// The kinds of image a chat takes: the ones both Pi-Bolt and Claude Code take in a message.
pub const IMAGE_KINDS: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// How many images one chat message can carry. A person pastes a screen at a time; this leaves room for a few.
pub const MAX_IMAGES: usize = 6;

/// How large one image of a chat message can be, before the harnesses' own size limit.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

impl ChatImage {
    /// The image as Pi's `prompt` takes it: its bytes in base64, with the kind beside them.
    pub fn base64(&self) -> String {
        STANDARD.encode(&self.bytes)
    }

    /// The image as Claude Code's `stream-json` takes it: a data URL of its kind.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64())
    }

    /// What a chat shows for this image in place of its bytes while its agent is still answering: the name the
    /// person's browser gave it, or a plain one of the kind.
    pub fn shown_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            format!(
                "image.{}",
                image_extension(&self.media_type).unwrap_or("bin")
            )
        })
    }

    /// The name the chat keeps this image under in its own directory: what its bytes hold, with the extension of
    /// its kind. An image of a kind Moebius does not keep has no name.
    pub fn file_name(&self) -> Option<String> {
        Some(format!(
            "{}.{}",
            hex::encode(Sha256::digest(&self.bytes)),
            image_extension(&self.media_type)?
        ))
    }

    /// What the chat's agent is given of this image besides its bytes: its kind, and a name to hold it by.
    pub fn described(&self) -> serde_json::Value {
        json!({
            "name": self.shown_name(),
            "media_type": self.media_type,
            "file": self.file_name(),
        })
    }
}

/// The extension a chat keeps an image of each kind under; none for a kind it does not keep.
fn image_extension(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Which kind of image a chat keeps under each extension, which is what its address answers as.
pub fn image_kind(extension: &str) -> Option<&'static str> {
    match extension {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Refuses a chat message carrying more than [`MAX_IMAGES`] images, one larger than [`MAX_IMAGE_BYTES`], one of a
/// kind outside [`IMAGE_KINDS`], or one whose bytes are not the picture its kind says. Whether the model can look at
/// them is `App::start_chat`'s, once the model is known.
pub fn check_images(images: &[ChatImage]) -> Result<()> {
    if images.len() > MAX_IMAGES {
        return Err(AppError::Invalid(format!(
            "a chat message takes at most {MAX_IMAGES} images, not {}",
            images.len()
        )));
    }
    for image in images {
        if image.bytes.is_empty() {
            return Err(AppError::Invalid(format!(
                "image {} is empty",
                image.shown_name()
            )));
        }
        if image.bytes.len() > MAX_IMAGE_BYTES {
            return Err(AppError::Invalid(format!(
                "image {} is larger than {MAX_IMAGE_BYTES} bytes",
                image.shown_name()
            )));
        }
        if !IMAGE_KINDS.contains(&image.media_type.as_str()) {
            return Err(AppError::Invalid(format!(
                "a chat takes {} and not {}",
                IMAGE_KINDS.join(", "),
                image.media_type
            )));
        }
        // The chat keeps the bytes under the kind it was told and answers them back as that kind, so they have to be
        // what it is said they are.
        if !bytes_are_of_kind(&image.media_type, &image.bytes) {
            return Err(AppError::Invalid(format!(
                "image {} is no image: its bytes are not a {}",
                image.shown_name(),
                image.media_type
            )));
        }
    }
    Ok(())
}

/// Whether the bytes begin as a picture of `media_type` does. Every kind a chat takes begins with a few bytes that
/// say which it is, which is also what a person's clipboard can get wrong.
fn bytes_are_of_kind(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/png" => bytes.starts_with(b"\x89PNG"),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => bytes.starts_with(b"GIF8"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
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

/// The model the agent `name` runs on, as its definition has it; none when it does not name one.
fn agent_model(runs: &dyn Runs, name: &str) -> Option<String> {
    runs.agents()
        .into_iter()
        .find(|agent| agent.name == name && !agent.model.is_empty())
        .map(|agent| agent.model)
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
        images: &[ChatImage],
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
        check_images(images)?;
        if !images.is_empty() && message.is_none() {
            return Err(AppError::Invalid(
                "images travel with a message, which cannot be empty".into(),
            ));
        }
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
        // A picture only goes to a chat whose model looks at images; the person is told to choose one that does.
        if !images.is_empty() && !self.takes_images(runs, agent, settings.model.as_deref()) {
            return Err(AppError::Invalid(format!(
                "{} takes words only: send the message without its images, or choose a model that looks at them",
                settings.model.as_deref().unwrap_or(agent)
            )));
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
            images,
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
    /// stays. The chat must be waiting, or have ended because its agent went away: while its agent works the answer is
    /// a conflict, and a chat that ended any other way is ended for good. The change is kept as a `settings_changed`
    /// event, so that the next message is answered with it: the chat's agent is told at once when it is up (see
    /// [`Runs::change_chat`]), and starts with what is stored when it is made again for the message (see
    /// [`App::wake_chat`]). `served` are the protocols the router's upstreams list `model` under.
    pub fn change_chat_settings(
        self: &Arc<Self>,
        runs: &dyn Runs,
        run_id: i64,
        (model, thinking, served): (Option<&str>, Option<&str>, &[Protocol]),
    ) -> Result<Run> {
        let run = self.chat(run_id)?;
        check_settings(model, thinking)?;
        let _turn = self.chat_turns.lock().unwrap_or_else(|e| e.into_inner());
        let run = self.run(run.id)?;
        if run.status != "waiting" && run.status != "stopped" {
            // A chat whose agent went away is not ended for good: the agent made again for the next message starts on
            // what is stored, so such a chat is given its settings too.
            return Err(if run.ended_at.is_some() {
                AppError::Invalid(format!(
                    "chat {run_id} ended {}; its model and reasoning cannot change",
                    run.status
                ))
            } else {
                AppError::Conflict(format!(
                    "chat {run_id} is {}: its model and reasoning change once the agent has answered",
                    run.status
                ))
            });
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
        // Only a chat whose agent is up has an agent to tell.
        if run.status == "waiting" {
            runs.change_chat(self, run_id).map_err(AppError::Invalid)?;
        }
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

    /// A person writes to a chat. While its agent is up, the chat keeps the message and the pictures attached
    /// to it as a `said` event, and the agent answers it when it is done with what it is doing, told first where
    /// the person is when `context` is given. A chat that ended because its agent went away is taken up again
    /// with what was said in it (see [`App::wake_chat`]), so that the person carries on in the chat they are
    /// reading instead of starting a new one. A chat that ended any other way is ended for good: it is
    /// continued by a new chat (see [`App::start_chat`]).
    pub fn say_in_chat(
        self: &Arc<Self>,
        runs: &dyn Runs,
        run_id: i64,
        message: &str,
        images: &[ChatImage],
        context: Option<&str>,
    ) -> Result<Run> {
        self.chat(run_id)?;
        if message.trim().is_empty() {
            return Err(AppError::Invalid("a message cannot be blank".into()));
        }
        check_images(images)?;
        if self.terminals.is_open(tui_key(run_id)) {
            return Err(AppError::Conflict(format!(
                "chat {run_id} is in its terminal: close it to write to the chat"
            )));
        }
        let turn = self.chat_turns.lock().unwrap_or_else(|e| e.into_inner());
        // Read under the lock, so that two messages to an ended chat do not both take it up.
        let run = self.chat(run_id)?;
        // The pictures go to the model with the words, so a chat whose model cannot look at them says so before it
        // keeps the message.
        if !images.is_empty()
            && !self.takes_images(
                runs,
                &run.agent,
                self.chat_settings(run_id)?.model.as_deref(),
            )
        {
            return Err(AppError::Invalid(
                "this chat's model takes words only: send the message without its images, or change the model to \
                 one that looks at them"
                    .into(),
            ));
        }
        if self.is_waking(run_id) {
            return Err(Self::waking_conflict(run_id));
        }
        if run.ended_at.is_some() {
            return self.wake_chat(runs, turn, &run, message, images, context);
        }
        // Stored first, so that it comes before the agent's records of its answer.
        let described = self.store_chat_images(run_id, images);
        self.record_run_event(
            run_id,
            "said",
            &json!({ "message": redact(message), "images": described }),
        )?;
        // The chat works from now on, before its agent says so, so that its worktree is not offered meanwhile.
        self.store.set_chat_working(run_id)?;
        if !runs.say(run_id, &with_context(context, message), images) {
            return Err(AppError::Invalid(format!("chat {run_id} has ended")));
        }
        Ok(run)
    }

    /// A person writes to the ended chat `run`, which ended because its agent went away (a person pressing Stop or the
    /// server stopping alike): the agent starts again in the chat's own place and on its own branch, told what was said
    /// in the chat before, and the chat works from now on. What the person wrote is the message it answers, told first
    /// where they are when `context` is given. `turn` is the hold on `chat_turns`, kept until the chat is taken up and
    /// let go before its agent starts, so that other chats are not kept waiting. When the agent cannot start again the
    /// chat is left as it was, and why it cannot is the error.
    fn wake_chat(
        self: &Arc<Self>,
        runs: &dyn Runs,
        turn: MutexGuard<'_, ()>,
        run: &Run,
        message: &str,
        images: &[ChatImage],
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
        // What can be told beforehand is, so that a message that cannot be answered is not kept in the chat.
        self.check_chat_branch(run)
            .map_err(|why| AppError::Invalid(format!("chat {run_id} cannot start again: {why}")))?;
        let told = continues_prompt(&self.conversation_of(run_id)?, message);
        // Stored first, so that it comes before the agent's records of its answer.
        let described = self.store_chat_images(run_id, images);
        self.record_run_event(
            run_id,
            "said",
            &json!({ "message": redact(message), "images": described }),
        )?;
        // The chat works from now on and is no longer ended, before its agent says so.
        let reopened = self.store.reopen_chat(run_id)?;
        if reopened {
            // Until the agent is registered with the harness, a message, a stop or an end cannot reach it.
            self.waking
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(run_id);
        }
        drop(turn);
        if !reopened {
            return Err(AppError::Invalid(format!("chat {run_id} has ended")));
        }
        let woken = runs.wake(self, run_id, &with_context(context, &told), images);
        if woken.is_err() {
            // As it was: ended because its agent went away, its worktree taken away again with it.
            let _ = self.store.set_run_status(run_id, "stopped");
        }
        self.waking
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&run_id);
        if let Err(e) = woken {
            return Err(AppError::Invalid(format!(
                "chat {run_id} cannot start again: {e}"
            )));
        }
        self.run(run_id)
    }

    /// Whether the chat `run_id` is being woken: its agent is starting and cannot be reached yet.
    pub(super) fn is_waking(&self, run_id: i64) -> bool {
        self.waking
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&run_id)
    }

    /// What a message, a stop or an end to the chat `run_id` gets while it is being woken.
    pub(super) fn waking_conflict(run_id: i64) -> AppError {
        AppError::Conflict(format!("chat {run_id} is starting; try again in a moment"))
    }

    /// Whether the branch of the stopped chat `run`, which its agent is started on again, is still there.
    fn check_chat_branch(&self, run: &Run) -> std::result::Result<(), String> {
        let clone = if let Some(repository) = run.repository_id {
            PathBuf::from(self.repository(repository).map_err(|e| e.to_string())?.path)
        } else if run.harness_repository {
            self.harness_repo()
                .map_err(|e| e.to_string())?
                .path()
                .to_path_buf()
        } else {
            return Ok(());
        };
        if self.repos.chat_branch_exists(&clone, run.id) {
            Ok(())
        } else {
            Err(format!(
                "the chat's branch moebius/chat-{} is gone: start a new chat to continue from what was said",
                run.id
            ))
        }
    }

    /// A person ends a chat that has not ended. It ends as `succeeded` once its agent has shut down and its work is
    /// committed and, if there is any, offered as a change (see `offer_chat_change`).
    pub fn end_chat(&self, runs: &dyn Runs, run_id: i64) -> Result<Run> {
        let run = self.running_chat(run_id)?;
        if !runs.end(run_id) {
            if self.is_waking(run_id) {
                return Err(Self::waking_conflict(run_id));
            }
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

    /// The directory holding the images the messages of the chat `run_id` carry. It is the chat's own, beside its
    /// transcript rather than inside a worktree, so a picture a person pastes changes no files of the repository.
    pub fn chat_image_dir(&self, run_id: i64) -> PathBuf {
        self.data_dir
            .join("chats")
            .join(run_id.to_string())
            .join("images")
    }

    /// Keeps a copy of each of the `images` a chat message carries under the chat's own directory, named by what its
    /// bytes hold, and answers what the message's `said` event says of them. The bytes do not go into the event,
    /// which holds JSON text; the app serves them back from the address each entry names.
    ///
    /// A copy that cannot be written leaves the message whole: the agent is handed the picture either way, and the
    /// chat shows the picture's name where it would otherwise show the picture.
    pub fn store_chat_images(&self, run_id: i64, images: &[ChatImage]) -> serde_json::Value {
        let dir = self.chat_image_dir(run_id);
        let mut described = Vec::new();
        for image in images {
            if let Some(file) = image.file_name() {
                let path = dir.join(&file);
                if !path.is_file() {
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::fs::write(&path, &image.bytes);
                }
            }
            described.push(image.described());
        }
        json!(described)
    }

    /// The bytes of the image `file` the chat `run_id` keeps, and which kind of image it is. Nothing when `file` is
    /// not a name Moebius writes, so no address can reach outside the chat's own directory.
    pub fn read_chat_image(&self, run_id: i64, file: &str) -> Option<(Vec<u8>, &'static str)> {
        let (hash, extension) = file.split_once('.')?;
        let kind = image_kind(extension)?;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = std::fs::read(self.chat_image_dir(run_id).join(file)).ok()?;
        Some((bytes, kind))
    }

    /// Whether a chat on `model`, or on the model its agent `agent` has when the chat names none, looks at images
    /// along with its words. A chat whose model Moebius cannot work out is given the images: what the model itself
    /// says of them is likelier to be true than a guess here, and the harness reports it.
    pub fn takes_images(&self, runs: &dyn Runs, agent: &str, model: Option<&str>) -> bool {
        let known = model
            .map(str::to_string)
            .or_else(|| agent_model(runs, agent))
            .filter(|model| !model.is_empty());
        match known {
            Some(model) => runs.takes_images(&model),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use moebius_plugin_api::Host;

    use super::asks;
    use crate::app::{App, AppError, ChatSettings};
    use crate::board::{AgentInfo, CardRuns, ChatRuns, ImproveRuns, StartError, Variant};
    use crate::decide::Actor;
    use crate::harness::remote::RunnerInfo;
    use crate::store::{Card, Run, Store};

    /// A harness whose agents are never up: only `wake` does anything, and it blocks until it is let go.
    struct SlowWake {
        entered: Mutex<Sender<()>>,
        release: Mutex<Receiver<Result<(), String>>>,
    }

    impl CardRuns for SlowWake {
        fn start(
            &self,
            _: &Arc<App>,
            _: &Card,
            _: &str,
            _: (&str, Option<&str>),
            _: Variant,
        ) -> Result<Run, StartError> {
            unimplemented!()
        }
        fn has_agent(&self, _: &str) -> bool {
            false
        }
        fn runners(&self) -> Vec<RunnerInfo> {
            Vec::new()
        }
        fn agents(&self) -> Vec<AgentInfo> {
            Vec::new()
        }
        fn token_budget(&self, _: &str) -> Option<i64> {
            None
        }
        fn steer(&self, _: i64, _: &str) -> bool {
            false
        }
        fn stop(&self, _: i64) -> bool {
            false
        }
    }

    impl ChatRuns for SlowWake {
        fn tui(&self, _: &Arc<App>, _: i64) -> Result<std::process::Command, String> {
            unimplemented!()
        }
        fn close_tui(&self, _: &Arc<App>, _: i64) {}
        fn start_chat(
            &self,
            _: &Arc<App>,
            _: &str,
            _: (Option<&crate::store::Repository>, bool),
            _: Option<&str>,
            _: &[ChatImage],
            _: Option<&str>,
            _: &str,
            _: &ChatSettings,
        ) -> Result<Run, StartError> {
            unimplemented!()
        }
        fn change_chat(&self, _: &Arc<App>, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn say(&self, _: i64, _: &str, _: &[ChatImage]) -> bool {
            false
        }
        fn takes_images(&self, _: &str) -> bool {
            true
        }
        fn wake(
            &self,
            app: &Arc<App>,
            run_id: i64,
            _: &str,
            _: &[ChatImage],
        ) -> Result<(), String> {
            self.entered.lock().unwrap().send(()).unwrap();
            let outcome = self.release.lock().unwrap().recv().unwrap();
            if outcome.is_ok() {
                app.store.set_run_status(run_id, "running").unwrap();
            }
            outcome
        }
        fn end(&self, _: i64) -> bool {
            false
        }
    }

    impl ImproveRuns for SlowWake {
        fn start_retro(
            &self,
            _: &Arc<App>,
            _: &std::path::Path,
            _: &str,
            _: &str,
            _: &std::collections::BTreeMap<String, String>,
        ) -> Result<Run, StartError> {
            unimplemented!()
        }
        fn start_research(
            &self,
            _: &Arc<App>,
            _: &crate::store::Repository,
            _: &str,
            _: &str,
        ) -> Result<Run, StartError> {
            unimplemented!()
        }
        fn start_trial(
            &self,
            _: &Arc<App>,
            _: &Card,
            _: &str,
            _: crate::config::AgentConfig,
            _: i64,
            _: &str,
        ) -> Result<Run, StartError> {
            unimplemented!()
        }
    }

    fn said(app: &App, run_id: i64) -> usize {
        app.store
            .events(run_id)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "said")
            .count()
    }

    #[test]
    fn a_chat_being_woken_answers_a_message_a_stop_and_an_end_with_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::in_memory(&dir.path().join("blobs")).unwrap());
        let host = Arc::new(Host::new(Duration::from_secs(1)));
        let app = Arc::new(App::new(store, host, &dir.path().join("data")));
        let (entered, waiting) = channel();
        let (release, released) = channel();
        let runs = Arc::new(SlowWake {
            entered: Mutex::new(entered),
            release: Mutex::new(released),
        });
        let chat = app.store.add_run(None, "Coder", "fake", "chat").unwrap();
        app.store.set_run_status(chat.id, "stopped").unwrap();

        let waker = {
            let (app, runs) = (app.clone(), runs.clone());
            std::thread::spawn(move || app.say_in_chat(&*runs, chat.id, "Wake up", &[], None))
        };
        waiting.recv().unwrap();
        // The chat is reopened, and its agent is not up yet.
        assert_eq!(said(&app, chat.id), 1);
        let conflict = |error: AppError| {
            assert!(
                matches!(&error, AppError::Conflict(why) if why.contains("is starting")),
                "{error}"
            );
        };
        conflict(
            app.say_in_chat(&*runs, chat.id, "Hello", &[], None)
                .unwrap_err(),
        );
        conflict(
            app.stop_run(&Actor::Person, &*runs, chat.id, "")
                .unwrap_err(),
        );
        conflict(app.end_chat(&*runs, chat.id).unwrap_err());
        assert_eq!(said(&app, chat.id), 1);

        release.send(Ok(())).unwrap();
        waker.join().unwrap().unwrap();
        // Up now: a message takes the normal path (this fake's agent refuses it, which is not a conflict).
        let why = app
            .say_in_chat(&*runs, chat.id, "Hello", &[], None)
            .unwrap_err();
        assert!(matches!(why, AppError::Invalid(_)), "{why}");

        // A wake that fails lets go as well, and leaves the chat stopped.
        app.store.set_run_status(chat.id, "stopped").unwrap();
        let waker = {
            let (app, runs) = (app.clone(), runs.clone());
            std::thread::spawn(move || app.say_in_chat(&*runs, chat.id, "Again", &[], None))
        };
        waiting.recv().unwrap();
        conflict(
            app.say_in_chat(&*runs, chat.id, "Hello", &[], None)
                .unwrap_err(),
        );
        release.send(Err("no room".into())).unwrap();
        assert!(waker.join().unwrap().is_err());
        assert!(!app.is_waking(chat.id));
        assert_eq!(app.run(chat.id).unwrap().status, "stopped");
    }

    use super::{
        ChatImage, IMAGE_KINDS, MAX_IMAGE_BYTES, MAX_IMAGES, bytes_are_of_kind, check_images,
        image_extension, image_kind,
    };

    /// A picture a message carries, of the kind it says and holding what it is given.
    fn picture(name: Option<&str>, media_type: &str, bytes: &[u8]) -> ChatImage {
        ChatImage {
            name: name.map(str::to_string),
            media_type: media_type.to_string(),
            bytes: bytes.to_vec(),
        }
    }

    /// The bytes a PNG begins with, which is enough for these tests: Moebius keeps a picture's bytes as they are.
    const PNG_HEAD: &[u8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn a_message_carries_a_few_pictures_of_the_kinds_the_harnesses_take() {
        assert!(check_images(&[]).is_ok());
        assert!(check_images(&[picture(None, "image/png", PNG_HEAD)]).is_ok());

        // One picture more than a message carries.
        let many = vec![picture(None, "image/png", PNG_HEAD); MAX_IMAGES + 1];
        let why = check_images(&many).unwrap_err().to_string();
        assert!(why.contains(&format!("at most {MAX_IMAGES}")), "{why}");

        // A kind neither harness takes.
        let why = check_images(&[picture(Some("scan.tiff"), "image/tiff", b"II*\0")])
            .unwrap_err()
            .to_string();
        assert!(why.contains(&IMAGE_KINDS.join(", ")), "{why}");
        assert!(why.contains("image/tiff"), "{why}");

        // Bytes that are no picture of the kind they are sent as.
        let why = check_images(&[picture(
            Some("notes.png"),
            "image/png",
            b"notes, not a picture",
        )])
        .unwrap_err()
        .to_string();
        assert!(why.contains("is no image"), "{why}");
        assert!(why.contains("notes.png"), "{why}");

        // A picture larger than a chat keeps, and none at all.
        let big = vec![0u8; MAX_IMAGE_BYTES + 1];
        let why = check_images(&[picture(None, "image/png", &big)])
            .unwrap_err()
            .to_string();
        assert!(why.contains(&format!("{MAX_IMAGE_BYTES}")), "{why}");
        let why = check_images(&[picture(None, "image/png", &[])])
            .unwrap_err()
            .to_string();
        assert!(why.contains("empty"), "{why}");
    }

    #[test]
    fn each_kind_a_chat_keeps_begins_with_bytes_that_say_which_it_is() {
        assert!(bytes_are_of_kind("image/png", b"\x89PNG\r\n\x1a\nrest"));
        assert!(bytes_are_of_kind("image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(bytes_are_of_kind("image/gif", b"GIF89a\0"));
        assert!(bytes_are_of_kind("image/webp", b"RIFF\0\0\0\0WEBPVP8 "));
        // A video of the same container, and a picture of another kind sent under this one's name.
        assert!(!bytes_are_of_kind("image/webp", b"RIFF\0\0\0\0AVI "));
        assert!(!bytes_are_of_kind("image/png", b"GIF89a\0"));
        assert!(!bytes_are_of_kind("image/png", b""));
    }

    #[test]
    fn a_picture_is_kept_by_what_it_holds_and_shown_by_what_it_is_called() {
        let kept = picture(Some("Screenshot of the board.png"), "image/png", PNG_HEAD);
        let file = kept.file_name().unwrap();
        assert!(file.ends_with(".png"), "{file}");
        assert_eq!(file.len(), 64 + ".png".len(), "{file}");
        // The same bytes again under another name is the same picture kept once.
        assert_eq!(
            picture(Some("again.png"), "image/png", PNG_HEAD)
                .file_name()
                .unwrap(),
            file
        );
        assert_eq!(kept.described()["name"], "Screenshot of the board.png");
        assert_eq!(kept.described()["media_type"], "image/png");
        assert_eq!(kept.described()["file"], file);

        // A picture the clipboard gave no name is shown by its kind, and goes to a harness as its own bytes.
        let unnamed = picture(None, "image/jpeg", &[0xFF, 0xD8, 0xFF]);
        assert_eq!(unnamed.shown_name(), "image.jpg");
        assert_eq!(unnamed.base64(), "/9j/");
        assert_eq!(unnamed.data_url(), "data:image/jpeg;base64,/9j/");

        assert_eq!(image_extension("image/gif"), Some("gif"));
        assert_eq!(image_extension("image/svg+xml"), None);
        assert_eq!(image_kind("jpg"), Some("image/jpeg"));
        assert_eq!(image_kind("exe"), None);
    }

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
