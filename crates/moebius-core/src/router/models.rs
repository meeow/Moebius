//! The models the upstreams serve, asked of all of them at once for the pickers of the desktop app and kept for a
//! while, without a key or a password in the answer, the model classes the router serves besides them, and whether
//! any of them takes images along with its text.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Protocol, Router, Upstream};
use crate::server::setup::without_credentials;

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
    /// Whether a chat running on this model can take images with a message.
    pub images: bool,
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
    /// The models the upstreams serve, asked of all of them at once. The answer is kept for five minutes unless
    /// `refresh` is set. An upstream that fails is listed in `errors`; no key is ever in the answer. The classes are
    /// the router's at the time of asking.
    pub async fn available_models(&self, refresh: bool) -> AvailableModels {
        let mut answer = self.listed_models(refresh).await;
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
        let asked = futures_util::future::join_all(upstreams.iter().map(|u| self.list_models(u)));
        let mut answer = AvailableModels::default();
        for (upstream, result) in upstreams.iter().zip(asked.await) {
            match result {
                Ok(ids) => answer
                    .models
                    .extend(ids.into_iter().map(|id| AvailableModel {
                        images: model_takes_images(&id),
                        id,
                        protocol: upstream.protocol,
                        upstream: without_credentials(&upstream.base_url),
                    })),
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
        self.list_models(&Upstream {
            protocol: Protocol::OpenAi,
            base_url: base_url.to_string(),
            api_key: key,
            oauth: None,
        })
        .await
    }

    /// The ids of the models one upstream lists, or a short message saying why not.
    pub(super) async fn list_models(&self, upstream: &Upstream) -> Result<Vec<String>, String> {
        if let Some(needs) = upstream.oauth.as_ref().and_then(|oauth| oauth.needs()) {
            return Err(format!("needs {needs}"));
        }
        if let Some(oauth) = upstream.oauth.as_ref().filter(|oauth| oauth.codex()) {
            // The backend has no list to ask for: the models are the ones its subscription serves.
            self.access_token(oauth).await?;
            return Ok(super::responses::MODELS
                .iter()
                .map(|id| id.to_string())
                .collect());
        }
        let base = upstream.base_url.trim_end_matches('/');
        let request = match upstream.protocol {
            Protocol::OpenAi => self.client.get(format!("{base}/models")),
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
            .filter_map(|m| m["id"].as_str().map(str::to_string))
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
mod tests {
    use super::model_takes_images;

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
