//! The chats: a person talking with an agent, in a repository, the harness configuration repository or a directory
//! of its own, and what grilling chats ended in.
//!
//! - `POST /v1/chats` `{agent?, message?, images?, repository?, continues?, model?, thinking?}`: starts a chat, with
//!   the Moebius agent unless `agent` names another, or goes on from the ended chat `continues`; without a message it
//!   waits for the first. `GET /v1/chats`: the chats, newest first. `POST /v1/chats/{id}/settings` `{model?,
//!   thinking?}`: changes the model or reasoning level of a waiting chat.
//! - `POST /v1/chats/{id}/messages` `{message, images?}`: writes to a chat. `POST /v1/chats/{id}/offer` `{reason?}`:
//!   offers what a chat's worktree changes as a change without ending the chat. `POST /v1/chats/{id}/end`: ends a
//!   chat. `POST /v1/chats/{id}/seen` `{seq}`: a person has read a chat that far.
//! - `GET /v1/chats/{id}/images/{file}`: a picture a chat message carried, which the chat keeps under itself; the
//!   name is the one the message's `said` event gives.
//! - `GET /v1/grillings` and `GET /v1/grillings/{id}`: what grilling chats ended in. `POST /v1/grillings/{id}/confirm`
//!   `{issues: [{title, body, blocked_by}]}` makes a card of each issue the person kept; `POST
//!   /v1/grillings/{id}/dismiss` drops the proposal or the report (see [`crate::harness::grilling`]).

use axum::Extension;
use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use super::{ApiError, ApiState, Reason, blocking, internal};
use crate::app::{Chat, ChatImage, MAX_IMAGE_BYTES};
use crate::decide::Actor;
use crate::harness::grilling;
use crate::store::{Change, Grilling, GrillingIssue, Run};

/// The routes of the chats and the grillings.
pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/v1/chats", get(chats).post(start_chat))
        .route("/v1/chats/{id}/messages", post(say_in_chat))
        .route("/v1/chats/{id}/offer", post(offer_chat_changes))
        .route("/v1/chats/{id}/end", post(end_chat))
        .route("/v1/chats/{id}/seen", post(chat_seen))
        .route("/v1/chats/{id}/settings", post(change_chat_settings))
        .route("/v1/chats/{id}/images/{file}", get(chat_image))
        .route("/v1/grillings", get(grillings))
        .route("/v1/grillings/{id}", get(get_grilling))
        .route("/v1/grillings/{id}/confirm", post(confirm_grilling))
        .route("/v1/grillings/{id}/dismiss", post(dismiss_grilling))
}

/// The body that starts a chat: the agent to talk to (the Moebius agent without one), the first message (without one
/// the chat waits for it) and the pictures attached to it, and optionally the repository, or the harness
/// configuration repository, to work in, and the model and reasoning level to use in place of the agent's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewChat {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    /// The pictures the first message carries, which its model has to look at.
    #[serde(default)]
    pub images: Vec<NewChatImage>,
    /// The model the chat runs on in place of its agent's.
    #[serde(default)]
    pub model: Option<String>,
    /// The reasoning level the chat runs with in place of its agent's: `off`, `minimal`, `low`, `medium`, `high`,
    /// `xhigh` or `max`.
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub repository: Option<i64>,
    /// Whether the chat works in the harness configuration repository instead of a repository.
    #[serde(default)]
    pub harness: bool,
    /// An ended chat this one continues: its agent and place unless others are named, and its conversation.
    #[serde(default)]
    pub continues: Option<i64>,
    /// Where the person is in the app, which the agent is told with the message and the chat does not show.
    #[serde(default)]
    pub context: Option<String>,
    /// Where the chat is started: `chat` (the default) or `assistant`, the desktop app's floating window, whose chats
    /// `GET /v1/chats` leaves out unless asked.
    #[serde(default)]
    pub origin: Option<String>,
}

/// The body that changes what a waiting chat runs with: a model or a reasoning level, each kept when not given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewChatSettings {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
}

/// The body of a message to a chat: what the person wrote, which the chat shows, the pictures attached to it, and
/// optionally where they are in the app, which only the agent is told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub message: String,
    /// The pictures the message carries, which the chat's model has to look at.
    #[serde(default)]
    pub images: Vec<NewChatImage>,
    #[serde(default)]
    pub context: Option<String>,
}

/// An image a person attaches to a chat message: what their browser or clipboard called it, which kind of image it
/// is, and its bytes in base64, which is how the harnesses take them. A chat takes the kinds of
/// [`crate::app::IMAGE_KINDS`], at most [`crate::app::MAX_IMAGES`] to one message and each of at most
/// [`MAX_IMAGE_BYTES`] bytes, and only on a model that looks at images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewChatImage {
    #[serde(default)]
    pub name: Option<String>,
    pub media_type: String,
    pub data: String,
}

impl NewChatImage {
    /// The image as the core takes it, its bytes decoded. A body holding more than the base64 of the largest image a
    /// chat takes, or no base64 at all, is refused here rather than at a harness; what the bytes hold, and which kind
    /// the body claims they are, the core checks once they are decoded.
    fn decoded(self) -> Result<ChatImage, ApiError> {
        // Base64 takes four characters for every three bytes, so this many characters cannot hold too many bytes.
        if self.data.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 {
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("a chat image is at most {MAX_IMAGE_BYTES} bytes"),
            ));
        }
        let bytes = STANDARD.decode(self.data.as_bytes()).map_err(|_| {
            ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "image {} is not base64",
                    self.name.as_deref().unwrap_or("of a chat message")
                ),
            )
        })?;
        Ok(ChatImage {
            name: self.name,
            media_type: self.media_type,
            bytes,
        })
    }
}

/// The images of a request body, decoded for the core. One image that cannot be decoded fails the whole message:
/// a person's pictures are sent together or not at all.
fn decode_images(images: Vec<NewChatImage>) -> Result<Vec<ChatImage>, ApiError> {
    images
        .into_iter()
        .map(NewChatImage::decoded)
        .collect::<Result<Vec<_>, _>>()
}

/// Which chats to list: with `all`, also those of the desktop app's floating window.
#[derive(Debug, Deserialize)]
struct ChatsQuery {
    #[serde(default)]
    all: bool,
}

async fn grillings(State(state): State<ApiState>) -> Result<axum::Json<Vec<Grilling>>, ApiError> {
    blocking(&state.app, |app| Ok(app.store.grillings()?)).await
}

async fn get_grilling(
    State(state): State<ApiState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<axum::Json<Grilling>, ApiError> {
    blocking(&state.app, move |app| grilling::view(app, id)).await
}

/// The body of `POST /v1/grillings/{id}/confirm`: the issues as the person left them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedIssues {
    pub issues: Vec<GrillingIssue>,
}

async fn confirm_grilling(
    State(state): State<ApiState>,
    Extension(by): Extension<Actor>,
    UrlPath(id): UrlPath<i64>,
    axum::Json(confirmed): axum::Json<ConfirmedIssues>,
) -> Result<axum::Json<Grilling>, ApiError> {
    blocking(&state.app, move |app| {
        grilling::confirm(app, &by, id, confirmed.issues)
    })
    .await
}

async fn dismiss_grilling(
    State(state): State<ApiState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<axum::Json<Grilling>, ApiError> {
    blocking(&state.app, move |app| grilling::dismiss(app, id)).await
}

/// The protocols the router's upstreams list `model` under, or the models of the class it names; none without a model.
async fn served(state: &ApiState, model: Option<&str>) -> Vec<crate::router::Protocol> {
    let Some(model) = model.map(str::trim) else {
        return Vec::new();
    };
    let available = state.router.available_models(false).await;
    // A model class is served in the protocols of its models.
    if let Some(class) = available.classes.iter().find(|class| class.name == model) {
        return class.protocols.clone();
    }
    available
        .models
        .iter()
        .filter(|available| available.id == model)
        .map(|available| available.protocol)
        .collect()
}

async fn start_chat(
    State(state): State<ApiState>,
    axum::Json(chat): axum::Json<NewChat>,
) -> Result<axum::Json<Run>, ApiError> {
    let runs = state.runs.clone();
    let served = served(&state, chat.model.as_deref()).await;
    let images = decode_images(chat.images)?;
    blocking(&state.app, move |app| {
        app.start_chat(
            &*runs,
            chat.agent.as_deref(),
            chat.repository,
            chat.harness,
            chat.message.as_deref(),
            &images,
            chat.context.as_deref(),
            chat.continues,
            chat.origin.as_deref(),
            (chat.model.as_deref(), chat.thinking.as_deref(), &served),
        )
    })
    .await
}

async fn change_chat_settings(
    State(state): State<ApiState>,
    UrlPath(id): UrlPath<i64>,
    axum::Json(settings): axum::Json<NewChatSettings>,
) -> Result<axum::Json<Run>, ApiError> {
    let runs = state.runs.clone();
    let served = served(&state, settings.model.as_deref()).await;
    blocking(&state.app, move |app| {
        let chosen = (
            settings.model.as_deref(),
            settings.thinking.as_deref(),
            served.as_slice(),
        );
        app.change_chat_settings(&*runs, id, chosen)
    })
    .await
}

async fn chats(
    State(state): State<ApiState>,
    Query(query): Query<ChatsQuery>,
) -> Result<axum::Json<Vec<Chat>>, ApiError> {
    blocking(&state.app, move |app| app.chats(query.all)).await
}

async fn say_in_chat(
    State(state): State<ApiState>,
    UrlPath(id): UrlPath<i64>,
    axum::Json(said): axum::Json<ChatMessage>,
) -> Result<axum::Json<Run>, ApiError> {
    let runs = state.runs.clone();
    let images = decode_images(said.images)?;
    blocking(&state.app, move |app| {
        app.say_in_chat(&*runs, id, &said.message, &images, said.context.as_deref())
    })
    .await
}

async fn offer_chat_changes(
    State(state): State<ApiState>,
    Extension(by): Extension<Actor>,
    UrlPath(id): UrlPath<i64>,
    body: Option<axum::Json<Reason>>,
) -> Result<axum::Json<Change>, ApiError> {
    let reason = body.map(|body| body.0.reason).unwrap_or_default();
    blocking(&state.app, move |app| {
        app.offer_chat_changes(&by, id, &reason)
    })
    .await
}

async fn end_chat(
    State(state): State<ApiState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<axum::Json<Run>, ApiError> {
    let runs = state.runs.clone();
    blocking(&state.app, move |app| app.end_chat(&*runs, id)).await
}

/// How far a person has read a chat.
#[derive(Debug, Deserialize)]
struct Seen {
    seq: i64,
}

async fn chat_seen(
    State(state): State<ApiState>,
    UrlPath(id): UrlPath<i64>,
    axum::Json(seen): axum::Json<Seen>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    blocking(&state.app, move |app| {
        app.mark_chat_seen(id, seen.seq)
            .map(|()| serde_json::json!({}))
    })
    .await
}

/// `GET /v1/chats/{id}/images/{file}`: one of the pictures a chat's messages carried, which the chat keeps under
/// itself. The name is the one the message's `said` event gives, `<sha256>.<extension>`, so no address reaches
/// outside the chat, and a picture never changes once it is sent.
async fn chat_image(
    State(state): State<ApiState>,
    UrlPath((id, file)): UrlPath<(i64, String)>,
) -> Result<axum::response::Response, ApiError> {
    let app = state.app.clone();
    let named = file.clone();
    let kept = tokio::task::spawn_blocking(move || app.read_chat_image(id, &file))
        .await
        .map_err(internal)?;
    let Some((bytes, media_type)) = kept else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("chat {id} keeps no image {named}"),
        ));
    };
    Ok((
        [
            (header::CONTENT_TYPE, media_type.to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
// Some tests are Unix only; on Windows what only they use is unused.
#[cfg_attr(not(unix), allow(unused))]
mod tests {
    use super::*;
    use crate::app::{IMAGE_KINDS, MAX_IMAGES};
    use crate::repos::git;
    use crate::server::api::ApiState;
    use crate::server::api::testing::{
        agent, chat, end, get, kinds, origin, post, say, start, start_with, status, until,
    };
    use crate::store::Event;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A picture of one pixel, which is what these tests send as base64, and its bytes.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

    /// The body of a message's picture: what it is called, which kind it claims to be, and its bytes in base64.
    fn picture_body(name: &str, media_type: &str, data: &str) -> serde_json::Value {
        json!({ "name": name, "media_type": media_type, "data": data })
    }

    /// `GET /v1/chats/{id}/images/{file}`, without asking for JSON.
    async fn kept_picture(
        state: &ApiState,
        addr: &str,
        run_id: i64,
        file: &str,
    ) -> reqwest::Response {
        reqwest::Client::new()
            .get(format!(
                "http://{addr}/v1/chats/{run_id}/images/{}",
                urlencoding(file)
            ))
            .bearer_auth(&*state.token)
            .send()
            .await
            .unwrap()
    }

    /// A path segment as the route takes it; `{file}` is one segment, so a name with a slash in it is what it reads.
    fn urlencoding(name: &str) -> String {
        name.bytes()
            .map(|b| match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                    String::from(b as char)
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    /// The agents these tests use: one whose model looks at pictures and one that takes words only.
    fn looking() -> BTreeMap<String, crate::config::AgentConfig> {
        BTreeMap::from([("Vision".to_string(), agent("gpt-4o"))])
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_answers_two_messages_in_a_row_and_keeps_its_history() {
        let (_dir, state, addr) = start().await;
        let app = &state.app;
        let run = chat(&state, &addr, "Hello", None).await;
        assert_eq!(
            kinds(app, run.id),
            [
                "said",
                "response",
                "agent_start",
                "message_update",
                "agent_end",
                "agent_settled"
            ]
        );
        // Without a repository the agent works in an empty directory of its own.
        let empty = app.data_dir.join(format!("chats/{}", run.id));
        assert!(empty.is_dir());

        let said = say(&state, &addr, run.id, "Again").await;
        assert_eq!(said.status(), 200);
        let settled = |n: usize| {
            kinds(app, run.id)
                .iter()
                .filter(|k| *k == "agent_settled")
                .count()
                == n
                && status(app, run.id) == "waiting"
        };
        until("the second answer", || settled(2)).await;
        let events: Vec<Event> = get(&state, &addr, &format!("/v1/runs/{}/events", run.id)).await;
        let words: Vec<(&str, &str)> = events
            .iter()
            .filter_map(|e| match e.kind.as_str() {
                "said" => Some(("said", e.data["message"].as_str()?)),
                "message_update" => Some((
                    "answered",
                    e.data["assistantMessageEvent"]["delta"].as_str()?,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            words,
            [
                ("said", "Hello"),
                ("answered", "You said: Hello"),
                ("said", "Again"),
                ("answered", "You said: Again"),
            ]
        );

        // What a person says is stored without secrets.
        say(
            &state,
            &addr,
            run.id,
            "My key is sk-ant-abcdefghijklmnop1234",
        )
        .await;
        until("the third answer", || settled(3)).await;
        let third = app.store.events(run.id).unwrap();
        let said = third.iter().rfind(|e| e.kind == "said").unwrap();
        assert_eq!(said.data["message"], "My key is [redacted:anthropic-key]");

        let other = chat(&state, &addr, "Another chat", None).await;
        let listed: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
        assert_eq!(
            listed.iter().map(|c| c.id).collect::<Vec<_>>(),
            [other.id, run.id]
        );
        assert_eq!(
            (
                listed[1].agent.as_str(),
                listed[1].status.as_str(),
                listed[1].title.as_str()
            ),
            ("Coder", "waiting", "Hello")
        );
        assert_eq!(listed[1].repository_id, None);

        // A person ends the chat; its empty directory goes with it.
        assert_eq!(end(&state, &addr, run.id).await.status(), 200);
        until("the chat ends", || status(app, run.id) == "succeeded").await;
        assert!(!empty.exists());
        assert_eq!(end(&state, &addr, other.id).await.status(), 200);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_keeps_the_pictures_its_messages_carry_and_gives_them_back() {
        let (_dir, state, addr) = start_with(looking()).await;
        let app = &state.app;
        let started = post(
            &state,
            &addr,
            "/v1/chats".into(),
            json!({
                "agent": "Vision",
                "message": "What is in this picture?",
                "images": [picture_body("screenshot.png", "image/png", PNG)],
            }),
        )
        .await;
        assert_eq!(started.status(), 200, "{}", started.status());
        let run: Run = started.json().await.unwrap();
        until("the agent has answered", || {
            status(app, run.id) == "waiting"
                && kinds(app, run.id).iter().any(|k| k == "agent_settled")
        })
        .await;

        // The chat shows the picture by what it is called and where it is kept, never by its bytes.
        let said = app
            .store
            .events(run.id)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "said")
            .unwrap();
        assert_eq!(said.data["images"][0]["name"], "screenshot.png");
        assert_eq!(said.data["images"][0]["media_type"], "image/png");
        let file = said.data["images"][0]["file"].as_str().unwrap().to_string();
        assert!(file.ends_with(".png"), "{file}");
        assert_eq!(file.len(), 64 + ".png".len(), "{file}");
        assert!(app.chat_image_dir(run.id).join(&file).is_file());

        // The bytes come back as they went in, under the kind they were sent as.
        let kept = kept_picture(&state, &addr, run.id, &file).await;
        assert_eq!(kept.status(), 200);
        assert_eq!(kept.headers()["content-type"], "image/png");
        assert_eq!(
            kept.bytes().await.unwrap().as_ref(),
            base64::engine::general_purpose::STANDARD
                .decode(PNG)
                .unwrap()
                .as_slice()
        );

        // A second message goes to the same picture, and the chat keeps it once.
        let said = post(
            &state,
            &addr,
            format!("/v1/chats/{}/messages", run.id),
            json!({
                "message": "And in this one?",
                "images": [picture_body("again.png", "image/png", PNG)],
            }),
        )
        .await;
        assert_eq!(said.status(), 200);
        until("the second answer", || {
            kinds(app, run.id)
                .iter()
                .filter(|k| *k == "agent_settled")
                .count()
                == 2
        })
        .await;
        assert!(app.chat_image_dir(run.id).join(&file).is_file());

        // The chat gives out no picture it does not keep, and no name Moebius did not write.
        assert_eq!(
            kept_picture(&state, &addr, run.id, "notes.txt")
                .await
                .status(),
            404
        );
        assert_eq!(
            kept_picture(&state, &addr, run.id, "../config")
                .await
                .status(),
            404
        );
        assert_eq!(kept_picture(&state, &addr, run.id, "").await.status(), 404);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_refuses_pictures_its_model_does_not_look_at() {
        // The Coder's model is `fake`, which Moebius does not know to look at images.
        let (_dir, state, addr) = start().await;
        let refused = post(
            &state,
            &addr,
            "/v1/chats".into(),
            json!({
                "agent": "Coder",
                "message": "Look at this",
                "images": [picture_body("shot.png", "image/png", PNG)],
            }),
        )
        .await;
        assert_eq!(refused.status(), 422);
        assert!(
            refused.text().await.unwrap().contains("takes words only"),
            "the chat does not say why it refuses"
        );

        // The same for a message to a chat already open, which goes on as if the picture was never sent.
        let run = chat(&state, &addr, "Hello", None).await;
        let refused = post(
            &state,
            &addr,
            format!("/v1/chats/{}/messages", run.id),
            json!({
                "message": "Look at this",
                "images": [picture_body("shot.png", "image/png", PNG)],
            }),
        )
        .await;
        assert_eq!(refused.status(), 422);
        assert!(refused.text().await.unwrap().contains("takes words only"));
        assert_eq!(
            kinds(&state.app, run.id)
                .iter()
                .filter(|k| **k == "said")
                .count(),
            1
        );

        // A chat named a model that looks at images takes them, and one changed to a model that does not is asked to
        // choose again.
        let (_dir, state, addr) = start_with(looking()).await;
        let run = post(
            &state,
            &addr,
            "/v1/chats".into(),
            json!({
                "agent": "Vision",
                "message": "What is in this picture?",
                "images": [picture_body("shot.png", "image/png", PNG)],
            }),
        )
        .await;
        assert_eq!(run.status(), 200);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_takes_only_the_pictures_it_can_show() {
        let (_dir, state, addr) = start_with(looking()).await;
        let start = |body: serde_json::Value| {
            let state = state.clone();
            let addr = addr.clone();
            async move {
                let response = post(&state, &addr, "/v1/chats".into(), body).await;
                (response.status(), response.text().await.unwrap_or_default())
            }
        };

        // A message is needed for a picture to travel with.
        let (status, why) = start(json!({
            "agent": "Vision",
            "images": [picture_body("shot.png", "image/png", PNG)],
        }))
        .await;
        assert_eq!(status, 422);
        assert!(why.contains("travel with a message"), "{why}");

        // One more picture than a chat takes.
        let (status, why) = start(json!({
            "agent": "Vision",
            "message": "Many pictures",
            "images": (0..=MAX_IMAGES)
                .map(|n| picture_body(&format!("shot{n}.png"), "image/png", PNG))
                .collect::<Vec<_>>(),
        }))
        .await;
        assert_eq!(status, 422);
        assert!(why.contains("at most"), "{why}");

        // A kind of picture a chat does not take, even named with a picture's extension.
        let (status, why) = start(json!({
            "agent": "Vision",
            "message": "A drawing",
            "images": [picture_body("shot.tiff", "image/tiff", PNG)],
        }))
        .await;
        assert_eq!(status, 422);
        assert!(why.contains(&IMAGE_KINDS.join(", ")), "{why}");

        // Bytes that are no picture's at all, sent as the picture its name claims.
        let (status, why) = start(json!({
            "agent": "Vision",
            "message": "Notes",
            "images": [picture_body(
                "notes.png",
                "image/png",
                &STANDARD.encode("notes, not a picture"),
            )],
        }))
        .await;
        assert_eq!(status, 422);
        assert!(why.contains("is no image"), "{why}");

        // Bytes too large for a chat, named as a picture.
        let (status, why) = start(json!({
            "agent": "Vision",
            "message": "A scan",
            "images": [picture_body(
                "scan.png",
                "image/png",
                &"A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 8)
            )],
        }))
        .await;
        assert_eq!(status, 413, "{why}");

        // Bytes that are not base64 at all.
        let (status, why) = start(json!({
            "agent": "Vision",
            "message": "A scan",
            "images": [picture_body("scan.png", "image/png", "not base64 at all !!!")],
        }))
        .await;
        assert_eq!(status, 422, "{why}");
        assert!(why.contains("is not base64"), "{why}");
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_in_a_repository_commits_its_work_on_its_branch_when_it_ends() {
        let (dir, state, addr) = start().await;
        let app = &state.app;
        let repo = app
            .add_repository("demo", origin(dir.path()).to_str().unwrap())
            .unwrap();
        let clone = std::path::PathBuf::from(&repo.path);
        let run = chat(&state, &addr, "Make notes [write notes.txt]", Some(repo.id)).await;
        let worktree = app.store.run(run.id).unwrap().unwrap().worktree.unwrap();
        let worktree = std::path::PathBuf::from(worktree);
        assert!(worktree.join("notes.txt").is_file());
        assert_eq!(run.branch(), format!("moebius/chat-{}", run.id));

        assert_eq!(end(&state, &addr, run.id).await.status(), 200);
        until("the chat ends", || status(app, run.id) == "succeeded").await;
        assert!(!worktree.exists());
        assert_eq!(
            git(&clone, &["show", &format!("{}:notes.txt", run.branch())]).unwrap(),
            "written"
        );
        let listed: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
        assert_eq!(listed[0].repository_id, Some(repo.id));

        // A chat that changed nothing has still succeeded; its branch stays where it started.
        let quiet = chat(&state, &addr, "Hello", Some(repo.id)).await;
        assert_eq!(end(&state, &addr, quiet.id).await.status(), 200);
        until("the chat ends", || status(app, quiet.id) == "succeeded").await;
        assert_eq!(
            git(&clone, &["rev-parse", &quiet.branch()]).unwrap(),
            git(&clone, &["rev-parse", "main"]).unwrap()
        );

        // Nothing can be said to a chat that has ended, and it cannot be ended again.
        for id in [run.id, quiet.id] {
            assert_eq!(say(&state, &addr, id, "Hello?").await.status(), 422);
            assert_eq!(end(&state, &addr, id).await.status(), 422);
        }
        assert_eq!(
            kinds(app, run.id).iter().filter(|k| *k == "said").count(),
            1
        );
    }

    async fn offer(state: &ApiState, addr: &str, run_id: i64) -> reqwest::Response {
        post(
            state,
            addr,
            format!("/v1/chats/{run_id}/offer"),
            serde_json::json!({ "reason": "ready to look at" }),
        )
        .await
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_offers_its_changes_and_goes_on_and_a_second_offer_updates_the_change() {
        let (dir, state, addr) = start().await;
        let app = &state.app;
        let repo = app
            .add_repository("demo", origin(dir.path()).to_str().unwrap())
            .unwrap();
        let clone = std::path::PathBuf::from(&repo.path);
        let run = chat(&state, &addr, "Make notes [write notes.txt]", Some(repo.id)).await;
        let changed = || async {
            let listed: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
            listed
                .iter()
                .find(|c| c.id == run.id)
                .unwrap()
                .changed_files
        };
        assert_eq!(changed().await, 1);

        let offered = offer(&state, &addr, run.id).await;
        assert_eq!(offered.status(), 200);
        let change: Change = offered.json().await.unwrap();
        assert_eq!(
            (
                change.branch.as_str(),
                change.state.as_str(),
                change.card_id
            ),
            (run.branch().as_str(), "open", None)
        );
        assert_eq!(
            git(&clone, &["show", &format!("{}:notes.txt", run.branch())]).unwrap(),
            "written"
        );
        let items = |change: i64| {
            app.store
                .inbox()
                .unwrap()
                .into_iter()
                .filter(|item| item.kind == "ready_to_merge" && item.data["change"] == change)
                .count()
        };
        assert_eq!(items(change.id), 1);
        // The chat goes on in the same worktree and takes the next message.
        assert_eq!(status(app, run.id), "waiting");
        assert_eq!(changed().await, 1);
        assert_eq!(
            say(&state, &addr, run.id, "More [write more.txt]")
                .await
                .status(),
            200
        );
        until("the second answer", || {
            kinds(app, run.id)
                .iter()
                .filter(|k| *k == "agent_settled")
                .count()
                == 2
                && status(app, run.id) == "waiting"
        })
        .await;
        assert_eq!(changed().await, 2);

        // Offered again, the same change takes the newer commit.
        let again: Change = offer(&state, &addr, run.id).await.json().await.unwrap();
        assert_eq!(again.id, change.id);
        let head = git(&clone, &["rev-parse", &run.branch()]).unwrap();
        assert_eq!(again.head.as_deref(), Some(head.as_str()));
        assert_ne!(again.head, change.head);
        assert_eq!(items(change.id), 1);
        // Nothing moved since: the open change is the answer, and nothing new is made.
        let same: Change = offer(&state, &addr, run.id).await.json().await.unwrap();
        assert_eq!((same.id, same.head), (change.id, again.head.clone()));
        let judged: Vec<serde_json::Value> = get(
            &state,
            &addr,
            &format!("/v1/judgements?run={}&kind=offer_chat_changes", run.id),
        )
        .await;
        assert_eq!(judged.len(), 3);

        // Ending the chat offers nothing more.
        assert_eq!(end(&state, &addr, run.id).await.status(), 200);
        until("the chat ends", || status(app, run.id) == "succeeded").await;
        assert_eq!(app.store.changes(repo.id).unwrap().len(), 1);
        assert_eq!(changed().await, 0);
        assert_eq!(offer(&state, &addr, run.id).await.status(), 422);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_with_nothing_to_offer_or_at_work_is_refused() {
        let (dir, state, addr) = start().await;
        let app = &state.app;
        let repo = app
            .add_repository("demo", origin(dir.path()).to_str().unwrap())
            .unwrap();
        let quiet = chat(&state, &addr, "Hello", Some(repo.id)).await;
        let refused = offer(&state, &addr, quiet.id).await;
        assert_eq!(refused.status(), 422);
        assert!(refused.text().await.unwrap().contains("nothing to offer"));
        let nowhere = chat(&state, &addr, "Hello", None).await;
        assert_eq!(offer(&state, &addr, nowhere.id).await.status(), 422);

        let started = post(
            &state,
            &addr,
            "/v1/chats".into(),
            serde_json::json!({ "agent": "Coder", "message": "Work for ever [hang]", "repository": repo.id }),
        )
        .await;
        let working: Run = started.json().await.unwrap();
        until("the agent works", || {
            kinds(app, working.id).contains(&"agent_start".to_string())
        })
        .await;
        assert_eq!(offer(&state, &addr, working.id).await.status(), 409);
        assert!(app.store.changes(repo.id).unwrap().is_empty());
        for id in [quiet.id, nowhere.id, working.id] {
            let stop = post(
                &state,
                &addr,
                format!("/v1/runs/{id}/stop"),
                serde_json::json!({}),
            );
            assert_eq!(stop.await.status(), 200);
        }
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_that_is_stopped_ends_and_its_agent_is_gone() {
        let (_dir, state, addr) = start().await;
        let app = &state.app;
        let waiting = chat(&state, &addr, "Hello", None).await;
        let pid = std::fs::read_to_string(
            app.data_dir
                .join(format!("runs/{}/agent/pid.txt", waiting.id)),
        )
        .unwrap();
        let stop = |id: i64| {
            post(
                &state,
                &addr,
                format!("/v1/runs/{id}/stop"),
                serde_json::json!({}),
            )
        };
        assert_eq!(stop(waiting.id).await.status(), 200);
        until("the chat is stopped", || {
            status(app, waiting.id) == "stopped"
        })
        .await;
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        assert!(!app.data_dir.join(format!("chats/{}", waiting.id)).exists());
        assert_eq!(stop(waiting.id).await.status(), 422);
        assert_eq!(say(&state, &addr, waiting.id, "Hello?").await.status(), 422);

        // A chat whose agent is working takes a message as a follow-up, and a stop aborts the turn.
        let started = post(
            &state,
            &addr,
            "/v1/chats".into(),
            serde_json::json!({ "agent": "Coder", "message": "Work for ever [hang]" }),
        )
        .await;
        let working: Run = started.json().await.unwrap();
        until("the agent works", || {
            kinds(app, working.id).contains(&"agent_start".to_string())
        })
        .await;
        assert_eq!(status(app, working.id), "running");
        assert_eq!(
            say(&state, &addr, working.id, "And then this.")
                .await
                .status(),
            200
        );
        let queued = app
            .data_dir
            .join(format!("runs/{}/agent/queued.txt", working.id));
        until("Pi queues the message", || {
            app.store
                .events(working.id)
                .unwrap()
                .iter()
                .any(|e| e.data["data"]["disposition"] == "queued")
        })
        .await;
        assert_eq!(std::fs::read_to_string(queued).unwrap(), "And then this.");
        assert_eq!(stop(working.id).await.status(), 200);
        until("the chat is stopped", || {
            status(app, working.id) == "stopped"
        })
        .await;
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_is_unread_until_the_person_has_seen_its_answer() {
        let (_dir, state, addr) = start().await;
        let run = chat(&state, &addr, "Hello", None).await;
        let listed: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
        let answered = listed[0].answered;
        assert!(answered > 0);
        assert_eq!(listed[0].seen, 0);

        let seen = |seq: i64| {
            post(
                &state,
                &addr,
                format!("/v1/chats/{}/seen", run.id),
                serde_json::json!({ "seq": seq }),
            )
        };
        assert_eq!(seen(answered).await.status(), 200);
        let listed: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
        assert_eq!(listed[0].seen, answered);
        // Reading never goes back.
        assert_eq!(seen(1).await.status(), 200);
        let listed: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
        assert_eq!(listed[0].seen, answered);

        // The next answer is unread again.
        say(&state, &addr, run.id, "Again").await;
        until("the second answer", || {
            state.app.chats(false).unwrap()[0].answered > answered
        })
        .await;
        let chat = &state.app.chats(false).unwrap()[0];
        assert!(chat.answered > chat.seen);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_chats_of_the_floating_window_are_listed_only_when_asked_for() {
        let (_dir, state, addr) = start().await;
        let listed = chat(&state, &addr, "On the Chat page", None).await;
        let body = serde_json::json!({ "agent": "Coder", "message": "In the window", "origin": "assistant" });
        let started = post(&state, &addr, "/v1/chats".into(), body).await;
        assert_eq!(started.status(), 200);
        let assistant: Run = started.json().await.unwrap();
        assert_eq!(assistant.origin, "assistant");
        assert_eq!(state.app.run(assistant.id).unwrap().origin, "assistant");
        assert_eq!(state.app.run(listed.id).unwrap().origin, "chat");

        let chats: Vec<Chat> = get(&state, &addr, "/v1/chats").await;
        let ids: Vec<i64> = chats.iter().map(|chat| chat.id).collect();
        assert_eq!(ids, [listed.id]);
        assert_eq!(chats[0].origin, "chat");
        let all: Vec<Chat> = get(&state, &addr, "/v1/chats?all=true").await;
        let all: Vec<(i64, &str)> = all.iter().map(|c| (c.id, c.origin.as_str())).collect();
        assert_eq!(all, [(assistant.id, "assistant"), (listed.id, "chat")]);

        let unknown =
            serde_json::json!({ "agent": "Coder", "message": "Hi", "origin": "elsewhere" });
        let refused = post(&state, &addr, "/v1/chats".into(), unknown).await;
        assert_eq!(refused.status(), 422);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_ended_chat_goes_on_in_a_new_chat_that_is_told_what_was_said() {
        let (_dir, state, addr) = start().await;
        let app = &state.app;
        let first = chat(&state, &addr, "Hello", None).await;
        let go_on = serde_json::json!({ "message": "Go on", "continues": first.id });
        // A chat that has not ended is written to, not continued.
        let refused = post(&state, &addr, "/v1/chats".into(), go_on.clone()).await;
        assert_eq!(refused.status(), 422);
        assert_eq!(end(&state, &addr, first.id).await.status(), 200);
        until("the chat has ended", || {
            app.store.run(first.id).unwrap().unwrap().ended_at.is_some()
        })
        .await;

        let started = post(&state, &addr, "/v1/chats".into(), go_on).await;
        assert_eq!(started.status(), 200);
        let run: Run = started.json().await.unwrap();
        assert_eq!(run.agent, "Coder");
        until("the agent has answered", || {
            status(app, run.id) == "waiting"
        })
        .await;
        let events = app.store.events(run.id).unwrap();
        // The chat shows what the person wrote; the agent (the fake Pi answers with its prompt) was told the rest.
        let said: Vec<&str> = events
            .iter()
            .filter(|e| e.kind == "said")
            .filter_map(|e| e.data["message"].as_str())
            .collect();
        assert_eq!(said, ["Go on"]);
        let answer = events
            .iter()
            .filter(|e| e.kind == "agent_end")
            .find_map(|e| crate::harness::pibolt::final_answer(&e.data))
            .unwrap();
        assert!(
            answer.contains("Person: Hello")
                && answer.contains("Agent: You said: Hello")
                && answer.ends_with("Go on"),
            "{answer}"
        );
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_agent_is_told_where_the_person_is_and_the_chat_shows_only_the_message() {
        let (_dir, state, addr) = start().await;
        let app = &state.app;
        let body = serde_json::json!({
            "agent": "Coder",
            "message": "What is this?",
            "context": "Page: Board of widgets\nKey: sk-ant-abcdefghijklmnop1234",
        });
        let started = post(&state, &addr, "/v1/chats".into(), body).await;
        assert_eq!(started.status(), 200);
        let run: Run = started.json().await.unwrap();
        until("the agent has answered", || {
            status(app, run.id) == "waiting"
        })
        .await;
        let body = serde_json::json!({ "message": "And this?", "context": "Page: Rules" });
        let said = post(
            &state,
            &addr,
            format!("/v1/chats/{}/messages", run.id),
            body,
        )
        .await;
        assert_eq!(said.status(), 200);
        until("the second answer", || {
            kinds(app, run.id)
                .iter()
                .filter(|k| *k == "agent_settled")
                .count()
                == 2
        })
        .await;
        let events = app.store.events(run.id).unwrap();
        let said: Vec<&str> = events
            .iter()
            .filter(|e| e.kind == "said")
            .filter_map(|e| e.data["message"].as_str())
            .collect();
        assert_eq!(said, ["What is this?", "And this?"]);
        // The fake Pi answers with its prompt: the context came first, without the secret.
        let answers: Vec<String> = events
            .iter()
            .filter(|e| e.kind == "agent_end")
            .filter_map(|e| crate::harness::pibolt::final_answer(&e.data))
            .collect();
        assert_eq!(answers.len(), 2);
        assert!(
            answers[0].contains("Page: Board of widgets\nKey: [redacted:anthropic-key]")
                && answers[0].ends_with("\n\nWhat is this?")
                && !answers[0].contains("sk-ant-"),
            "{}",
            answers[0]
        );
        assert!(
            answers[1].contains("Page: Rules") && answers[1].ends_with("\n\nAnd this?"),
            "{}",
            answers[1]
        );
        let stored = serde_json::to_string(&events.iter().map(|e| &e.data).collect::<Vec<_>>());
        assert!(!stored.unwrap().contains("sk-ant-abcdefghijklmnop1234"));
    }

    #[tokio::test]
    async fn a_chat_needs_a_known_agent_a_message_and_a_chat_to_write_to() {
        let (_dir, state, addr) = start().await;
        for body in [
            serde_json::json!({ "agent": "Nobody", "message": "Hello" }),
            serde_json::json!({ "agent": "Coder", "message": "  " }),
        ] {
            assert_eq!(
                post(&state, &addr, "/v1/chats".into(), body).await.status(),
                422
            );
        }
        let unknown = serde_json::json!({ "agent": "Coder", "message": "Hello", "repository": 99 });
        assert_eq!(
            post(&state, &addr, "/v1/chats".into(), unknown)
                .await
                .status(),
            404
        );
        assert_eq!(say(&state, &addr, 99, "Hello").await.status(), 404);
        assert!(state.app.store.chats().unwrap().is_empty());

        // A run of a card is no chat.
        let run = state
            .app
            .store
            .add_run(Some(1), "Coder", "test", "code")
            .unwrap();
        assert_eq!(say(&state, &addr, run.id, "Hello").await.status(), 422);
        assert_eq!(end(&state, &addr, run.id).await.status(), 422);
    }

    // Unix only: the fake Pi is a Python script started by its `#!` line.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_chat_in_a_repository_offers_what_it_made_as_a_change_the_person_merges() {
        let (dir, state, addr) = start().await;
        let app = &state.app;
        let repo = app
            .add_repository("demo", origin(dir.path()).to_str().unwrap())
            .unwrap();
        let clone = std::path::PathBuf::from(&repo.path);
        let mut events = app.bus.subscribe();
        let write = "Write a script [write .moebius/ready.luau]";
        let started = serde_json::json!({ "message": write, "repository": repo.id });
        let run: Run = post(&state, &addr, "/v1/chats".into(), started)
            .await
            .json()
            .await
            .unwrap();
        until("the agent has answered", || {
            status(app, run.id) == "waiting"
        })
        .await;
        assert!(app.store.changes(repo.id).unwrap().is_empty());
        assert_eq!(end(&state, &addr, run.id).await.status(), 200);
        until("the chat ends", || status(app, run.id) == "succeeded").await;

        // The change has no card, is on the chat's branch and waits in the inbox.
        let changes = app.store.changes(repo.id).unwrap();
        assert_eq!(changes.len(), 1);
        let change = &changes[0];
        assert_eq!(
            (
                change.card_id,
                change.branch.as_str(),
                change.state.as_str()
            ),
            (None, run.branch().as_str(), "open")
        );
        assert_eq!(change.pull_request, None);
        let inbox = app.store.inbox().unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            (inbox[0].kind.as_str(), inbox[0].card_id),
            ("ready_to_merge", None)
        );
        assert_eq!(inbox[0].title, write);
        assert_eq!(inbox[0].data["change"], change.id);
        let mut offered = None;
        while let Ok(event) = events.try_recv() {
            if event.kind == "change_offered" {
                offered = Some(event.data["id"].clone());
            }
        }
        assert_eq!(offered, Some(serde_json::json!(change.id)));

        // Merging it puts the script on the default branch and takes the item out of the inbox.
        let merged = post(
            &state,
            &addr,
            format!("/v1/changes/{}/merge", change.id),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(merged.status(), 200);
        assert_eq!(
            git(&clone, &["show", "main:.moebius/ready.luau"]).unwrap(),
            "written"
        );
        assert_eq!(
            git(&clone, &["log", "-1", "--format=%s", "main"]).unwrap(),
            format!("Merge {}", run.branch())
        );
        assert!(app.store.inbox().unwrap().is_empty());
        assert_eq!(
            app.store.change(change.id).unwrap().unwrap().state,
            "merged"
        );

        // A chat that changed nothing offers nothing; a stopped chat offers what it made.
        let quiet = chat(&state, &addr, "Hello", Some(repo.id)).await;
        assert_eq!(end(&state, &addr, quiet.id).await.status(), 200);
        until("the chat ends", || status(app, quiet.id) == "succeeded").await;
        let writing = chat(&state, &addr, "Again [write notes.txt]", Some(repo.id)).await;
        let stop = format!("/v1/runs/{}/stop", writing.id);
        let stopped = post(&state, &addr, stop, serde_json::json!({})).await;
        assert_eq!(stopped.status(), 200);
        until("the chat is stopped", || {
            status(app, writing.id) == "stopped"
        })
        .await;
        let branches: Vec<_> = app
            .store
            .changes(repo.id)
            .unwrap()
            .into_iter()
            .map(|c| c.branch)
            .collect();
        assert_eq!(branches, [run.branch(), writing.branch()]);
    }
}
