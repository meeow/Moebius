//! The models the upstreams serve, asked of all of them at once for the pickers of the desktop app and kept for a
//! while, without a key or a password in the answer, the model classes the router serves besides them, and the
//! reasoning level names of each model: the names its endpoint says it takes, or the ones `[router.model_thinking]`
//! gives it when it does not say. A run's `models.json` is given the name to send for every level whose own name the
//! endpoint does not take, so that its harness can ask for a level the endpoint would not take: `xhigh` of an endpoint
//! whose hardest level is named so, or the hardest an endpoint has when it has no name for a level at all. It
//! also says which models take pictures along with their text, which is what a chat message can carry.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Protocol, Router, Upstream};
use crate::config::THINKING_LEVELS;
use crate::server::setup::without_credentials;

/// The reasoning level names by model: for each reasoning level of a model, the value its endpoint takes.
pub type ModelThinking = BTreeMap<String, BTreeMap<String, String>>;

/// How long one upstream has to list its models.
const MODELS_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a list of models is kept before the upstreams are asked again.
const MODELS_KEPT: Duration = Duration::from_secs(5 * 60);

/// One model an upstream serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableModel {
    pub id: String,
    pub protocol: Protocol,
    /// The base URL of the upstream that serves it.
    pub upstream: String,
    /// The reasoning level names its endpoint said it takes, easiest first; empty when the upstream does not say,
    /// which is what an Anthropic endpoint and most OpenAI ones do.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<String>,
    /// The Moebius reasoning levels to offer for it; empty when every level is. Filled in as the answer is given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_levels: Vec<String>,
    /// The value a run sends for each reasoning level whose own name the model does not take: what
    /// `[router.model_thinking]` gives, or else what its listed `levels` give. Filled in as the answer is given.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub thinking: BTreeMap<String, String>,
    /// Whether a chat running on this model can hand a picture to it along with its text.
    pub images: bool,
}

/// A model an upstream listed, with the reasoning level names its endpoint takes.
struct Listed {
    id: String,
    levels: Vec<String>,
}

/// An upstream that could not say which models it serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamError {
    pub upstream: String,
    pub message: String,
}

/// A model class of `[router.classes]`, which a request may name as a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableClass {
    pub name: String,
    /// Its models, in the order they are tried.
    pub models: Vec<String>,
    /// The protocols the upstreams list any of its models under.
    pub protocols: Vec<Protocol>,
    /// Whether every model it lists takes images, which is what a chat that names the class needs.
    pub images: bool,
}

/// The models of all upstreams, sorted by protocol and then id, the upstreams that could not be asked, and the model
/// classes by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableModels {
    pub models: Vec<AvailableModel>,
    pub errors: Vec<UpstreamError>,
    #[serde(default)]
    pub classes: Vec<AvailableClass>,
    /// The models the decision model's endpoint lists, when they were asked for.
    #[serde(default)]
    pub decision_models: Vec<String>,
}

impl Router {
    /// Takes the reasoning level names of the models that need them, for a router being built.
    pub fn with_model_thinking(self, maps: ModelThinking) -> Self {
        self.set_model_thinking(maps);
        self
    }

    /// Replaces the reasoning level names; the next run of a model is written with the new ones.
    pub fn set_model_thinking(&self, maps: ModelThinking) {
        *self
            .model_thinking
            .write()
            .unwrap_or_else(|p| p.into_inner()) = maps;
    }

    /// The reasoning level names `model`'s endpoint was last heard taking, from the last time its upstream answered,
    /// however long ago; empty before the upstreams have been asked, and for an endpoint that does not name them.
    /// When two upstreams serve the model, the first in the configuration is the one heard.
    pub fn model_levels(&self, model: &str) -> Vec<String> {
        let upstreams = self.upstreams.read().unwrap_or_else(|p| p.into_inner());
        let kept = self.levels.lock().unwrap_or_else(|p| p.into_inner());
        upstreams
            .iter()
            .find_map(|u| kept.get(&upstream_key(u))?.get(model))
            .cloned()
            .unwrap_or_default()
    }

    /// The value to send for each reasoning level of `model`: what `[router.model_thinking]` gives it when it names
    /// the model, or the names its own endpoint lists; none when neither is known, when it takes the levels' own
    /// names, or when `model` is a model class (a class is answered by whichever of its models is picked, so no one
    /// map is right for it).
    pub fn thinking_map(&self, model: &str) -> Option<BTreeMap<String, String>> {
        self.thinking_map_for(model, &self.model_levels(model))
    }

    /// [`Router::thinking_map`] for a model whose listed `levels` are known.
    fn thinking_map_for(&self, model: &str, levels: &[String]) -> Option<BTreeMap<String, String>> {
        let given = self
            .model_thinking
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(model)
            .cloned();
        given.or_else(|| {
            let map = thinking_map_of(levels);
            (!map.is_empty()).then_some(map)
        })
    }

    /// The Moebius reasoning levels to offer for `model` with its listed `levels`: only the ones its endpoint names
    /// (`none` for `off`) and the ones `[router.model_thinking]` gives a value; empty when every level is offered,
    /// which is when nothing is known of the model's levels.
    fn offered_levels(&self, model: &str, levels: &[String]) -> Vec<String> {
        if levels.is_empty() {
            return Vec::new();
        }
        let given = self
            .model_thinking
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(model)
            .cloned()
            .unwrap_or_default();
        THINKING_LEVELS
            .iter()
            .filter(|level| {
                let own = if **level == "off" { "none" } else { level };
                levels.iter().any(|named| named == *level || named == own)
                    || given.contains_key(**level)
            })
            .map(|level| (*level).to_string())
            .collect()
    }

    /// Asks the upstreams which models they serve, once as it starts, then every `period`, and again as soon as the
    /// upstreams are edited, so that a run's `models.json` has the level names its model's endpoint takes without
    /// anyone opening the Models page. Never returns.
    pub async fn keep_levels(&self, period: Duration) {
        let mut interval = tokio::time::interval(period);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                () = self.upstreams_changed.notified() => {}
            }
            self.listed_models(true).await;
        }
    }

    /// The models the upstreams serve, asked of all of them at once. The answer is kept for five minutes unless
    /// `refresh` is set. An upstream that fails is listed in `errors`; no key is ever in the answer. The classes are
    /// the router's at the time of asking.
    pub async fn available_models(&self, refresh: bool) -> AvailableModels {
        let mut answer = self.listed_models(refresh).await;
        // The levels are the kept ones, but what they come to depends on `[router.model_thinking]`, which may have
        // been edited since the list was made.
        for model in &mut answer.models {
            model.thinking_levels = self.offered_levels(&model.id, &model.levels);
            model.thinking = self
                .thinking_map_for(&model.id, &model.levels)
                .unwrap_or_default();
        }
        answer.classes = self
            .classes()
            .into_iter()
            .map(|(name, listed)| {
                let images = listed.iter().all(|model| model_takes_images(&model.model));
                let models: Vec<String> = listed.into_iter().map(|m| m.model).collect();
                let mut protocols = Vec::new();
                for listed in answer.models.iter().filter(|m| models.contains(&m.id)) {
                    if !protocols.contains(&listed.protocol) {
                        protocols.push(listed.protocol);
                    }
                }
                AvailableClass {
                    name,
                    models,
                    protocols,
                    images,
                }
            })
            .collect();
        answer
    }

    /// The models the upstreams list, kept for five minutes unless `refresh` is set.
    async fn listed_models(&self, refresh: bool) -> AvailableModels {
        if !refresh {
            let kept = self.available.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, models)) = kept.as_ref()
                && at.elapsed() < MODELS_KEPT
            {
                return models.clone();
            }
        }
        let mut upstreams = self
            .upstreams
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let mut seen = Vec::new();
        upstreams.retain(|u| {
            let new = !seen.contains(u);
            seen.push(u.clone());
            new
        });
        let asked = futures_util::future::join_all(
            upstreams.iter().map(|u| self.listed_at(u, "/v1/models")),
        );
        let mut answer = AvailableModels::default();
        for (upstream, result) in upstreams.iter().zip(asked.await) {
            match result {
                Ok(listed) => {
                    // Only an upstream that answered changes what is known of its levels.
                    self.levels
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(
                            upstream_key(upstream),
                            listed
                                .iter()
                                .map(|m| (m.id.clone(), m.levels.clone()))
                                .collect(),
                        );
                    answer
                        .models
                        .extend(listed.into_iter().map(|m| AvailableModel {
                            images: model_takes_images(&m.id),
                            id: m.id,
                            levels: m.levels,
                            thinking_levels: Vec::new(),
                            thinking: BTreeMap::new(),
                            protocol: upstream.protocol,
                            upstream: without_credentials(&upstream.base_url),
                        }));
                }
                Err(message) => answer.errors.push(UpstreamError {
                    upstream: without_credentials(&upstream.base_url),
                    message,
                }),
            }
        }
        answer.models.sort_by(|a, b| {
            (a.protocol.name(), &a.id, &a.upstream).cmp(&(b.protocol.name(), &b.id, &b.upstream))
        });
        answer.models.dedup();
        answer.errors.dedup();
        *self.available.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((Instant::now(), answer.clone()));
        answer
    }

    /// The ids of the models an OpenAI-compatible endpoint outside the router lists, such as the decision model's,
    /// asked with `key` and never kept.
    pub async fn endpoint_models(
        &self,
        base_url: &str,
        key: Option<String>,
    ) -> Result<Vec<String>, String> {
        Ok(self
            .listed_at(
                &Upstream {
                    protocol: Protocol::OpenAi,
                    base_url: base_url.to_string(),
                    api_key: key,
                    oauth: None,
                },
                "/models",
            )
            .await?
            .into_iter()
            .map(|listed| listed.id)
            .collect())
    }

    /// The ids of the models one upstream lists, or a short message saying why not.
    pub(super) async fn list_models(&self, upstream: &Upstream) -> Result<Vec<String>, String> {
        Ok(self
            .listed_at(upstream, "/v1/models")
            .await?
            .into_iter()
            .map(|listed| listed.id)
            .collect())
    }

    /// [`Router::list_models`], where an `openai` upstream's list is at `openai_path` after its base url: a router
    /// upstream's base has no `/v1`, the decision model's url is asked at `/models` whatever it ends in.
    async fn listed_at(
        &self,
        upstream: &Upstream,
        openai_path: &str,
    ) -> Result<Vec<Listed>, String> {
        if let Some(needs) = upstream.oauth.as_ref().and_then(|oauth| oauth.needs()) {
            return Err(format!("needs {needs}"));
        }
        if let Some(oauth) = upstream.oauth.as_ref().filter(|oauth| oauth.codex()) {
            // The backend has no list to ask for: the models are the ones its subscription serves.
            self.access_token(oauth).await?;
            return Ok(super::responses::MODELS
                .iter()
                .map(|id| Listed {
                    id: (*id).to_string(),
                    // The subscription's own models: Pi-Bolt knows their levels, and the backend is not asked.
                    levels: Vec::new(),
                })
                .collect());
        }
        let base = upstream.base_url.trim_end_matches('/');
        let request = match upstream.protocol {
            Protocol::OpenAi => self.client.get(format!("{base}{openai_path}")),
            // The default page of Anthropic's list is twenty models.
            Protocol::Anthropic => self
                .client
                .get(format!("{base}/v1/models?limit=1000"))
                .header("anthropic-version", "2023-06-01"),
        }
        .timeout(MODELS_TIMEOUT);
        let request = self.authenticate(upstream, request).await?;
        let request = match upstream.oauth.as_ref().and_then(|oauth| oauth.preset()) {
            Some(preset) => preset.dress(request, None),
            None => request,
        };
        // The messages are fixed words, so that neither the key nor the address can be in them.
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                "timed out".to_string()
            } else if e.is_connect() {
                "could not connect".to_string()
            } else {
                "request failed".to_string()
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(status.as_u16().to_string());
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| "the answer is not JSON".to_string())?;
        let data = body["data"]
            .as_array()
            .ok_or_else(|| "the answer has no model list".to_string())?;
        Ok(data
            .iter()
            .filter_map(|model| {
                let id = model["id"].as_str()?.to_string();
                // An endpoint of this server's own kind names the reasoning levels its chat template takes.
                let levels = model["capabilities"]["reasoning_efforts"]
                    .as_array()
                    .map(|named| {
                        named
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Listed { id, levels })
            })
            .collect())
    }

    /// Whether `model`, a model or a class, takes images along with its text. A class takes them only when every
    /// model it lists does, because a chat cannot say beforehand which of them answers.
    pub fn takes_images(&self, model: &str) -> bool {
        match self.classes().get(model) {
            Some(models) => models.iter().all(|model| model_takes_images(&model.model)),
            None => model_takes_images(model),
        }
    }
}

/// Whether the model named `model` takes images along with its text.
///
/// Upstream model listings do not say, so the families that do are known by name: Claude from 3 on, Gemini and
/// Gemma from 3 on, GPT-4o and every OpenAI chat model after it, the OpenAI o-series, Grok 4, Llama 4, Pixtral,
/// GLM models ending in `v`, and any model whose own name calls it a vision model (`vl`, `vision`, `omni`).
/// A model none of these covers is taken as text only, which is what a chat that is handed an image is told.
pub fn model_takes_images(model: &str) -> bool {
    // What an upstream serves is often qualified ("anthropic/claude-sonnet-4-5"); the family is the last part.
    let name = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    // A model that listens, speaks, embeds or draws does not read a picture along with a question.
    if [
        "embedding",
        "audio",
        "tts",
        "whisper",
        "transcribe",
        "dall",
        "imagen",
        "veo",
        "sora",
    ]
    .iter()
    .any(|skip| name.contains(skip))
    {
        return false;
    }
    if [
        "vision", "omni", "pixtral", "internvl", "cogvlm", "-vl", "_vl",
    ]
    .iter()
    .any(|mark| name.contains(mark))
    {
        return true;
    }
    if name.starts_with("claude") {
        // Claude takes images from 3 on; the first two and the instant variant answer in words alone.
        return !name.contains("claude-1")
            && !name.contains("claude-2")
            && !name.contains("instant");
    }
    if [
        "gemini", "gemma-3", "gemma-4", "llama-4", "llama4", "grok-4",
    ]
    .iter()
    .any(|family| name.starts_with(family))
    {
        return true;
    }
    // GPT-4o onward and ChatGPT's own models take images; gpt-4, gpt-oss and the image makers do not.
    if ["gpt-4o", "gpt-4.1", "gpt-4.5", "gpt-5", "gpt-6", "chatgpt"]
        .iter()
        .any(|family| name.starts_with(family))
    {
        return true;
    }
    // The o-series: o1 and every numbered model after it, with or without a suffix, but not a name that merely
    // starts with an o.
    if name.len() > 1 && name.starts_with('o') && name.as_bytes()[1].is_ascii_digit() {
        return true;
    }
    name.starts_with("glm-4") && name.ends_with('v')
}

#[cfg(test)]
mod vision_tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::store::Store;

    #[test]
    fn the_vision_families_take_images() {
        for model in [
            "claude-haiku-4-5",
            "anthropic/claude-sonnet-4-5",
            "claude-3-5-sonnet-20241022",
            "gemini-3.1-pro-preview",
            "google/gemini-3.1-pro-preview",
            "gpt-4o",
            "gpt-4.1-mini",
            "gpt-5.6-sol",
            "chatgpt-codex-connector",
            "o4-mini",
            "gpt-4-vision-preview",
            "grok-4",
            "llama-4-maverick",
            "pixtral-large-latest",
            "qwen3-vl-235b-a22b-instruct",
            "moonshotai/kimi-k2.5-vision",
            "zhipuai/glm-4.6v",
            "gemma-3-27b-it",
            "mistralai/mistral-small-3.2-24b-instruct-vision",
        ] {
            assert!(model_takes_images(model), "{model} takes images");
        }
    }

    #[tokio::test]
    async fn the_answer_says_which_models_take_pictures() {
        // An OpenAI upstream that lists one model the router knows how to draw with and one it does not.
        let answers = json!({ "data": [{ "id": "gpt-4o" }, { "id": "deepseek-chat" }] });
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get(move || {
                let answers = answers.clone();
                async move { axum::Json(answers) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        let dir = tempfile::tempdir().unwrap();
        let router = Router::new(
            Arc::new(Store::in_memory(dir.path()).unwrap()),
            vec![Upstream {
                protocol: Protocol::OpenAi,
                base_url: base,
                api_key: None,
                oauth: None,
            }],
        );
        let listed = router.available_models(true).await;
        let pictures: Vec<(String, bool)> = listed
            .models
            .iter()
            .map(|model| (model.id.clone(), model.images))
            .collect();
        assert_eq!(
            pictures,
            vec![
                ("deepseek-chat".to_string(), false),
                ("gpt-4o".to_string(), true),
            ]
        );
    }

    #[test]
    fn models_that_answer_in_words_alone_do_not() {
        for model in [
            "claude-2.1",
            "claude-instant-1.2",
            "gpt-4",
            "gpt-3.5-turbo",
            "gpt-oss-120b",
            "gpt-image-1",
            "text-embedding-3-large",
            "grok-3",
            "llama-3.3-70b-versatile",
            "deepseek-chat",
            "qwen3-32b",
            "mistral-large-latest",
            "kimi-k2-turbo-preview",
            "gemma-2-27b-it",
            "openchat-3.6-8b",
        ] {
            assert!(!model_takes_images(model), "{model} is text only");
        }
    }
}

/// What `upstream` is known by in the levels learned of it: its protocol and base URL.
pub(super) fn upstream_key(upstream: &Upstream) -> String {
    format!("{} {}", upstream.protocol.name(), upstream.base_url)
}

/// How hard a reasoning level name asks, `none` and `off` being the same quietest; none for a name that is not one
/// of the known levels.
fn rank(name: &str) -> Option<usize> {
    match name {
        "none" => Some(0),
        _ => THINKING_LEVELS.iter().position(|level| *level == name),
    }
}

/// The name to send to an endpoint that takes `levels`, for each reasoning level whose own name the endpoint does not
/// take: the listed name nearest to it in how hard it asks (the harder one when two are as near, so that a request
/// for effort is not answered with less than it names), `off` going to `none` or `off` when the endpoint has one and
/// to its quietest name otherwise. A level harder than any the endpoint names asks for the hardest it has. The names
/// of a list none of which is a known level are taken by place instead. Nothing is given for a level the endpoint
/// takes by its own name, nor for any level of an endpoint that lists none; the harness sends those names themselves.
fn thinking_map_of(levels: &[String]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(last) = levels.last() else {
        return map;
    };
    let known: Vec<(usize, &String)> = levels
        .iter()
        .filter_map(|named| Some((rank(named)?, named)))
        .collect();
    for (place, level) in THINKING_LEVELS.iter().enumerate() {
        if levels.iter().any(|named| named == level) {
            continue;
        }
        let takes = if known.is_empty() {
            levels.get(place).unwrap_or(last)
        } else if place == 0 {
            // `off` is only ever the quietest name: `none`, or else whatever the endpoint has least.
            known
                .iter()
                .min_by_key(|(rank, _)| *rank)
                .map(|(_, named)| *named)
                .unwrap_or(last)
        } else {
            // Another level never goes to `none`, which turns reasoning off, unless that is all there is.
            let asking = known.iter().filter(|(rank, _)| *rank > 0);
            let nearest = asking
                .min_by_key(|(rank, _)| (rank.abs_diff(place), usize::MAX - rank))
                .or_else(|| known.first());
            nearest.map(|(_, named)| *named).unwrap_or(last)
        };
        map.insert((*level).to_string(), takes.clone());
    }
    map
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::response::IntoResponse;
    use serde_json::json;

    use super::*;
    use crate::store::Store;

    /// An OpenAI upstream whose one model names the reasoning levels its endpoint takes.
    async fn upstream_that(levels: Value) -> (Router, Arc<Store>, tempfile::TempDir) {
        let answers = json!({
            "data": [{ "id": "one-model", "capabilities": { "reasoning_efforts": levels } }]
        });
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get(move || {
                let answers = answers.clone();
                async move { axum::Json(answers) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::in_memory(dir.path()).unwrap());
        let router = Router::new(
            store.clone(),
            vec![Upstream {
                protocol: Protocol::OpenAi,
                base_url: base,
                api_key: None,
                oauth: None,
            }],
        );
        (router, store, dir)
    }

    #[tokio::test]
    async fn the_levels_an_endpoint_lists_are_kept_and_send_their_names() {
        let (router, _store, _dir) =
            upstream_that(json!(["none", "minimal", "low", "medium", "xhigh"])).await;
        let listed = router.available_models(true).await;
        assert_eq!(
            listed.models[0].levels,
            ["none", "minimal", "low", "medium", "xhigh"]
        );
        assert_eq!(router.model_levels("one-model"), listed.models[0].levels);
        // The endpoint has no name of its own for these three, so each takes the name at its place, or the hardest.
        assert_eq!(
            router.thinking_map("one-model").unwrap(),
            [
                ("off".to_string(), "none".to_string()),
                ("high".to_string(), "xhigh".to_string()),
                ("max".to_string(), "xhigh".to_string()),
            ]
            .into_iter()
            .collect::<BTreeMap<String, String>>()
        );
        assert_eq!(router.thinking_map("another-model"), None);
    }

    #[tokio::test]
    async fn an_endpoint_that_takes_the_level_names_is_given_no_map() {
        let (router, _store, _dir) = upstream_that(json!([
            "off", "minimal", "low", "medium", "high", "xhigh", "max"
        ]))
        .await;
        router.available_models(true).await;
        assert_eq!(router.thinking_map("one-model"), None);
    }

    #[tokio::test]
    async fn an_endpoint_that_does_not_say_is_given_no_map() {
        let (router, _store, _dir) = upstream_that(Value::Null).await;
        router.available_models(true).await;
        assert!(router.model_levels("one-model").is_empty());
        assert_eq!(router.thinking_map("one-model"), None);
    }

    #[tokio::test]
    async fn the_names_written_in_the_configuration_win() {
        let (router, _store, _dir) = upstream_that(json!(["low", "medium"])).await;
        router.set_model_thinking(
            [(
                "one-model".to_string(),
                [("high".to_string(), "medium".to_string())]
                    .into_iter()
                    .collect(),
            )]
            .into_iter()
            .collect(),
        );
        router.available_models(true).await;
        assert_eq!(
            router.thinking_map("one-model").unwrap(),
            [("high".to_string(), "medium".to_string())]
                .into_iter()
                .collect::<BTreeMap<String, String>>()
        );
    }

    fn names(listed: &[&str]) -> Vec<String> {
        listed.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_shorter_list_than_the_levels_sends_its_nearest_name_and_its_hardest_for_everything_above()
    {
        let map = thinking_map_of(&names(&["low", "high"]));
        assert_eq!(map["off"], "low");
        assert_eq!(map["minimal"], "low");
        assert!(!map.contains_key("high"), "it takes the name itself");
        assert_eq!(map["xhigh"], "high");
        assert_eq!(map["max"], "high");
    }

    #[test]
    fn a_list_that_does_not_start_at_the_bottom_is_mapped_by_how_hard_each_name_asks() {
        let map = thinking_map_of(&names(&["low", "medium", "high"]));
        assert_eq!(map["off"], "low");
        assert_eq!(map["minimal"], "low");
        assert_eq!(map["xhigh"], "high");
        assert_eq!(map["max"], "high");
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn none_is_the_quietest_wherever_the_list_has_it() {
        let map = thinking_map_of(&names(&["low", "medium", "high", "none"]));
        assert_eq!(map["off"], "none");
        assert_eq!(map["minimal"], "low", "reasoning is not turned off for it");
    }

    #[test]
    fn names_that_are_not_known_levels_are_taken_by_place() {
        let map = thinking_map_of(&names(&["quiet", "loud"]));
        assert_eq!(map["off"], "quiet");
        assert_eq!(map["minimal"], "loud");
        assert_eq!(map["max"], "loud");
    }

    /// An OpenAI upstream that lists `one-model` with `levels` until it is told to fail.
    async fn upstream_that_can_fail(
        levels: Value,
    ) -> (
        Router,
        Arc<std::sync::atomic::AtomicBool>,
        tempfile::TempDir,
    ) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let failing = Arc::new(AtomicBool::new(false));
        let answers = json!({
            "data": [{ "id": "one-model", "capabilities": { "reasoning_efforts": levels } }]
        });
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get({
                let failing = failing.clone();
                move || {
                    let answers = answers.clone();
                    let failing = failing.clone();
                    async move {
                        if failing.load(Ordering::SeqCst) {
                            (axum::http::StatusCode::BAD_GATEWAY, "down").into_response()
                        } else {
                            axum::Json(answers).into_response()
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        let dir = tempfile::tempdir().unwrap();
        let router = Router::new(
            Arc::new(Store::in_memory(dir.path()).unwrap()),
            vec![Upstream {
                protocol: Protocol::OpenAi,
                base_url: base,
                api_key: None,
                oauth: None,
            }],
        );
        (router, failing, dir)
    }

    #[tokio::test]
    async fn a_failed_ask_keeps_the_levels_learned_before() {
        use std::sync::atomic::Ordering;
        let (router, failing, _dir) =
            upstream_that_can_fail(json!(["none", "low", "medium", "xhigh"])).await;
        router.available_models(true).await;
        let before = router.thinking_map("one-model").unwrap();
        failing.store(true, Ordering::SeqCst);
        let again = router.available_models(true).await;
        assert!(again.models.is_empty() && !again.errors.is_empty());
        assert_eq!(router.thinking_map("one-model").unwrap(), before);
        assert_eq!(router.model_levels("one-model").len(), 4);
    }

    #[tokio::test]
    async fn editing_the_upstreams_keeps_the_levels_of_those_that_stay_and_asks_again() {
        let (router, _failing, _dir) =
            upstream_that_can_fail(json!(["none", "low", "xhigh"])).await;
        router.available_models(true).await;
        let upstreams = router.upstreams.read().unwrap().clone();
        router.set_upstreams(upstreams.clone());
        assert_eq!(router.model_levels("one-model").len(), 3);
        // The edit woke the loop that asks again; its permit is there to be taken at once.
        router.upstreams_changed.notified().await;
        let moved = Upstream {
            base_url: "http://127.0.0.1:9".to_string(),
            ..upstreams[0].clone()
        };
        router.set_upstreams(vec![moved]);
        assert!(router.model_levels("one-model").is_empty());
        assert_eq!(router.thinking_map("one-model"), None);
    }

    #[tokio::test]
    async fn the_upstreams_are_asked_when_the_loop_starts_and_when_they_are_edited() {
        let (router, _failing, _dir) =
            upstream_that_can_fail(json!(["none", "low", "xhigh"])).await;
        let router = Arc::new(router);
        let looping = tokio::spawn({
            let router = router.clone();
            async move { router.keep_levels(Duration::from_secs(3600)).await }
        });
        // Nobody asked for the list: the loop did.
        while router.model_levels("one-model").is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Editing the upstreams to another address with the same model: the loop asks that one too.
        let first = router.upstreams.read().unwrap()[0].clone();
        router.set_upstreams(vec![Upstream {
            base_url: format!("{}/", first.base_url),
            ..first
        }]);
        assert!(router.model_levels("one-model").is_empty());
        while router.model_levels("one-model").is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        looping.abort();
    }

    #[tokio::test]
    async fn the_answer_gives_the_effective_names_and_the_levels_to_offer() {
        let (router, _store, _dir) =
            upstream_that(json!(["none", "minimal", "low", "medium", "xhigh"])).await;
        let answer = router.available_models(true).await;
        let model = &answer.models[0];
        assert_eq!(
            model.thinking_levels,
            ["off", "minimal", "low", "medium", "xhigh"]
        );
        assert_eq!(model.thinking["high"], "xhigh");
        assert_eq!(model.thinking["off"], "none");
        // An override is the map, and makes the level it names valid; the kept answer does not hide the edit.
        router.set_model_thinking(BTreeMap::from([(
            "one-model".to_string(),
            BTreeMap::from([("max".to_string(), "xhigh".to_string())]),
        )]));
        let answer = router.available_models(false).await;
        let model = &answer.models[0];
        assert_eq!(
            model.thinking,
            BTreeMap::from([("max".to_string(), "xhigh".to_string())])
        );
        assert_eq!(
            model.thinking_levels,
            ["off", "minimal", "low", "medium", "xhigh", "max"]
        );
    }

    #[tokio::test]
    async fn a_model_class_is_given_no_map_whatever_its_models_list() {
        let (router, _store, _dir) = upstream_that(json!(["none", "low", "xhigh"])).await;
        router.set_classes(
            [(
                "strong".to_string(),
                vec![crate::router::ClassModel {
                    model: "one-model".to_string(),
                    day_tokens: None,
                }],
            )]
            .into_iter()
            .collect(),
        );
        let answer = router.available_models(true).await;
        assert!(router.thinking_map("one-model").is_some());
        assert_eq!(router.thinking_map("strong"), None);
        assert!(router.model_levels("strong").is_empty());
        assert_eq!(answer.classes[0].name, "strong");
    }

    #[tokio::test]
    async fn the_decision_models_url_is_asked_at_models_whatever_it_ends_in() {
        let app = axum::Router::new().route(
            "/openai/models",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"data": [{"id": "m1"}]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/openai", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        let dir = tempfile::tempdir().unwrap();
        let router = Router::new(Arc::new(Store::in_memory(dir.path()).unwrap()), Vec::new());
        assert_eq!(router.endpoint_models(&url, None).await.unwrap(), ["m1"]);
    }
}
