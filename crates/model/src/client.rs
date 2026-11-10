//! Calls to OpenAI-compatible APIs.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use net::Net;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::timeout;

use ts_rs::TS;

use crate::ModelError;
use crate::pricing::Prices;
use crate::providers::Provider;
use crate::usage::Usage;

/// Where and how to call a provider.
#[derive(Debug, Clone)]
pub struct Access {
    /// Without a trailing slash.
    pub base_url: String,
    pub api_key: Option<String>,
    /// None for an address the user filled in, which is asked only what all models take.
    pub provider: Option<&'static Provider>,
}

/// A model the provider offers.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListedModel {
    pub name: String,
    /// Where its provider's list says them.
    pub prices: Option<Prices>,
}

/// The models the provider offers, sorted by name.
pub async fn list_models(net: &Net, access: &Access) -> Result<Vec<ListedModel>, ModelError> {
    #[derive(Deserialize)]
    struct Models {
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
        #[serde(default)]
        pricing: Value,
    }
    let request = net
        .http()
        .get(format!("{}/models", access.base_url))
        .bearer_auth(access.api_key.as_deref().ok_or(ModelError::MissingKey)?)
        .timeout(Duration::from_secs(20));
    let body = read(send(request, access).await?, access).await?;
    let models: Models = serde_json::from_str(&body).map_err(|e| {
        // Not serde's own words, which quote the value it could not take.
        ModelError::Broken(format!(
            "model list: {:?} error at line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        ))
    })?;
    let priced = access.provider.is_some_and(|p| p.priced_list);
    let mut models: Vec<ListedModel> = models
        .data
        .into_iter()
        .map(|m| ListedModel {
            prices: priced.then(|| listed_prices(&m.pricing)).flatten(),
            name: m.id,
        })
        .collect();
    models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(models)
}

/// Prices per million tokens from OpenRouter's dollars per token, in decimal strings; None for a
/// model priced otherwise, such as its router, at "-1".
fn listed_prices(pricing: &Value) -> Option<Prices> {
    let per_million = |key: &str| {
        let per_token: f64 = pricing[key].as_str()?.parse().ok()?;
        // To the digits the price fields keep, which 0.000002 * 1e6 = 1.9999999999999998 is not.
        (per_token >= 0.0).then(|| (per_token * 1e12).round() / 1e6)
    };
    Some(Prices {
        input: Some(per_million("prompt")?),
        cache: per_million("input_cache_read"),
        output: Some(per_million("completion")?),
    })
}

/// Gives up on a provider that has sent nothing for this long, however long its whole answer
/// takes: a model may think for minutes, streaming its thoughts as it goes.
const IDLE: Duration = Duration::from_secs(60);

/// The most of an error response read, in bytes.
const ERROR_BODY: usize = 64 * 1024;

/// What a model answered.
#[derive(Debug)]
pub struct Answer {
    /// May be empty, such as when the model spent all it may write on thinking.
    pub text: String,
    pub finish_reason: Option<Finish>,
    /// None when the provider did not say.
    pub usage: Option<Usage>,
}

/// Why a model stopped, as OpenAI-compatible APIs say. A reason of a provider's own is `Other`: its
/// words could be anything, even what was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    Stop,
    /// Out of room for the answer.
    #[serde(alias = "max_output_tokens")]
    Length,
    ContentFilter,
    ToolCalls,
    #[serde(other)]
    Other,
}

/// What the user is told of an answer cut off at `Finish::Length`, which callers turn down: it may
/// hold no more than the model's thinking, or a JSON that happens to parse.
pub const CUT_OFF: &str = "模型的回答过长，已被截断。请重试，或换用能写更长回答的模型。";

/// Asks the model for a one-word answer, which proves the address, the key and the model name.
pub async fn test(net: &Net, access: &Access, model: &str) -> Result<(), ModelError> {
    let prompt = Prompt {
        model,
        system: None,
        user: "请只回复一个字：好",
        json: false,
    };
    let answer = complete(net, access, &prompt).await?;
    if answer.text.trim().is_empty() {
        return Err(ModelError::NoAnswer {
            finish_reason: answer.finish_reason,
        });
    }
    Ok(())
}

/// Asks the model to do what `system` says with what `user` gives it.
pub async fn ask(
    net: &Net,
    access: &Access,
    model: &str,
    system: &str,
    user: &str,
    json: bool,
) -> Result<Answer, ModelError> {
    let prompt = Prompt {
        model,
        system: Some(system),
        user,
        json,
    };
    complete(net, access, &prompt).await
}

struct Prompt<'a> {
    model: &'a str,
    system: Option<&'a str>,
    user: &'a str,
    /// Whether the answer is to be a JSON object.
    json: bool,
}

/// The two OpenAI-compatible APIs a model may answer on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Api {
    Chat,
    Responses,
}

/// The addresses and models found in this run to answer on `/responses` alone
/// (docs/design.md#桌面端).
static ON_RESPONSES: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

fn on_responses() -> MutexGuard<'static, BTreeSet<(String, String)>> {
    ON_RESPONSES
        .lock()
        .unwrap_or_else(|_| unreachable!("nothing panics while holding the lock"))
}

/// Calls the API the model answers on; when the provider has no such API, the other, which it
/// then answers on from here on.
async fn complete(net: &Net, access: &Access, prompt: &Prompt<'_>) -> Result<Answer, ModelError> {
    let target = (access.base_url.clone(), prompt.model.to_owned());
    let (first, other) = if on_responses().contains(&target) {
        (Api::Responses, Api::Chat)
    } else {
        (Api::Chat, Api::Responses)
    };
    let missing = |e: &ModelError| {
        matches!(
            e,
            ModelError::Refused {
                status: 404 | 405 | 501,
                ..
            }
        )
    };
    let refused = match call(net, access, prompt, first).await {
        Err(e) if missing(&e) => e,
        answered => return answered,
    };
    let answer = match call(net, access, prompt, other).await {
        // Neither: most likely a wrong address or model name, which the first says.
        Err(e) if missing(&e) => return Err(refused),
        answered => answered?,
    };
    tracing::info!(address = access.base_url, model = prompt.model, api = ?other, "model answers on the other API");
    if other == Api::Responses {
        on_responses().insert(target);
    } else {
        on_responses().remove(&target);
    }
    Ok(answer)
}

/// Sends the request to `api` and puts the answer together. Streamed, as some models only think
/// that way and not all can stop thinking. What the provider wrote stays out of the errors, which
/// go to the log: it may be the user's letter.
async fn call(
    net: &Net,
    access: &Access,
    prompt: &Prompt<'_>,
    api: Api,
) -> Result<Answer, ModelError> {
    let (thinking, json_mode) = access
        .provider
        .map_or((None, false), |p| p.asking(prompt.model));
    let (path, body) = match api {
        Api::Chat => {
            let messages: Vec<Value> = prompt
                .system
                .map(|system| json!({"role": "system", "content": system}))
                .into_iter()
                .chain([json!({"role": "user", "content": prompt.user})])
                .collect();
            let mut body = json!({
                "model": prompt.model,
                "messages": messages,
                "stream": true,
                // A streamed reply leaves out the tokens it took unless asked.
                "stream_options": {"include_usage": true},
            });
            if let Some(thinking) = thinking {
                thinking.apply(&mut body);
            }
            if prompt.json && json_mode {
                body["response_format"] = json!({"type": "json_object"});
            }
            ("chat/completions", body)
        }
        Api::Responses => {
            let mut body = json!({"model": prompt.model, "input": prompt.user, "stream": true});
            if let Some(system) = prompt.system {
                body["instructions"] = system.into();
            }
            if prompt.json && json_mode {
                body["text"] = json!({"format": {"type": "json_object"}});
            }
            ("responses", body)
        }
    };
    let request = net
        .http()
        .post(format!("{}/{path}", access.base_url))
        .bearer_auth(access.api_key.as_deref().ok_or(ModelError::MissingKey)?)
        .json(&body);
    let response = send(request, access).await?;
    match api {
        Api::Chat => chat(response, access).await,
        Api::Responses => responses(response, access).await,
    }
}

#[derive(Deserialize)]
struct Failure {
    message: String,
}

/// A Chat Completions answer, in chunks.
async fn chat(response: reqwest::Response, access: &Access) -> Result<Answer, ModelError> {
    #[derive(Deserialize)]
    struct Chunk {
        #[serde(default)]
        choices: Vec<Choice>,
        /// Some providers, such as OpenRouter, report failing partway here, after a success status.
        error: Option<Failure>,
        usage: Option<Tokens>,
    }
    #[derive(Deserialize)]
    struct Choice {
        #[serde(default)]
        delta: Delta,
        finish_reason: Option<Finish>,
        /// Where Kimi reports the tokens.
        usage: Option<Tokens>,
    }
    #[derive(Default, Deserialize)]
    struct Delta {
        content: Option<String>,
    }
    /// Counted in whole or not at all: some count only the total.
    #[derive(Deserialize)]
    struct Tokens {
        prompt_tokens: Option<i64>,
        completion_tokens: Option<i64>,
        prompt_tokens_details: Option<Details>,
        /// DeepSeek's name for the cached tokens.
        prompt_cache_hit_tokens: Option<i64>,
        /// Kimi's.
        cached_tokens: Option<i64>,
    }
    #[derive(Deserialize)]
    struct Details {
        cached_tokens: Option<i64>,
    }
    let (mut text, mut finish_reason, mut tokens) = (String::new(), None, None);
    events(response, access, |data, n| {
        let chunk: Chunk = event(data, n)?;
        if let Some(failure) = chunk.error {
            return Err(ModelError::Failed(without_key(&failure.message, access)));
        }
        tokens = chunk.usage.or(tokens.take());
        // The last chunk may only count tokens.
        if let Some(choice) = chunk.choices.into_iter().next() {
            text.extend(choice.delta.content);
            finish_reason = choice.finish_reason.or(finish_reason);
            tokens = choice.usage.or(tokens.take());
        }
        Ok(())
    })
    .await?;
    // What the provider's review stopped is no answer, however much came before.
    if finish_reason == Some(Finish::ContentFilter) {
        return Err(ModelError::NoAnswer { finish_reason });
    }
    let usage = tokens.and_then(|t| {
        Some(Usage {
            input: t.prompt_tokens?,
            cached: t
                .prompt_tokens_details
                .and_then(|d| d.cached_tokens)
                .or(t.prompt_cache_hit_tokens)
                .or(t.cached_tokens)
                .unwrap_or(0),
            output: t.completion_tokens?,
        })
    });
    Ok(Answer {
        text,
        finish_reason,
        usage,
    })
}

/// A Responses answer, in events; those about anything but its text are passed over.
async fn responses(response: reqwest::Response, access: &Access) -> Result<Answer, ModelError> {
    #[derive(Deserialize)]
    #[serde(tag = "type")]
    enum Event {
        #[serde(rename = "response.output_text.delta")]
        Text { delta: String },
        #[serde(rename = "response.completed")]
        Completed { response: Ended },
        #[serde(rename = "response.incomplete")]
        Incomplete { response: Ended },
        #[serde(rename = "response.failed")]
        Failed { response: Failed },
        /// Failing apart from a response, such as when the request is wrong.
        #[serde(rename = "error")]
        Error { message: String },
        #[serde(other)]
        Other,
    }
    #[derive(Deserialize)]
    struct Ended {
        incomplete_details: Option<Incomplete>,
        usage: Option<Tokens>,
    }
    #[derive(Deserialize)]
    struct Incomplete {
        reason: Option<Finish>,
    }
    #[derive(Deserialize)]
    struct Failed {
        error: Option<Failure>,
    }
    #[derive(Deserialize)]
    struct Tokens {
        input_tokens: i64,
        output_tokens: i64,
        input_tokens_details: Option<Details>,
    }
    #[derive(Deserialize)]
    struct Details {
        cached_tokens: Option<i64>,
    }
    let mut answer = Answer {
        text: String::new(),
        finish_reason: None,
        usage: None,
    };
    let mut end = |ended: Ended, finish_reason: Finish| {
        answer.finish_reason = Some(finish_reason);
        answer.usage = ended.usage.map(|t| Usage {
            input: t.input_tokens,
            cached: t
                .input_tokens_details
                .and_then(|d| d.cached_tokens)
                .unwrap_or(0),
            output: t.output_tokens,
        });
    };
    events(response, access, |data, n| {
        match event(data, n)? {
            Event::Text { delta } => answer.text.push_str(&delta),
            Event::Completed { response } => end(response, Finish::Stop),
            Event::Incomplete { response } => {
                let reason = response.incomplete_details.as_ref().and_then(|d| d.reason);
                // Out of room is the callers' to judge, as on Chat Completions; cut short for
                // anything else, such as the provider's review, it is no answer.
                if reason != Some(Finish::Length) {
                    return Err(ModelError::NoAnswer {
                        finish_reason: Some(reason.unwrap_or(Finish::Other)),
                    });
                }
                end(response, Finish::Length);
            }
            Event::Failed { response } => {
                let message = response.error.map_or_else(
                    || "the response failed without saying why".to_owned(),
                    |e| e.message,
                );
                return Err(ModelError::Failed(without_key(&message, access)));
            }
            Event::Error { message } => {
                return Err(ModelError::Failed(without_key(&message, access)));
            }
            Event::Other => {}
        }
        Ok(())
    })
    .await?;
    // Unlike Chat Completions, the end is always told: a stream closed before it was cut off.
    if answer.finish_reason.is_none() {
        return Err(ModelError::Failed(
            "the stream ended before the response did".into(),
        ));
    }
    Ok(answer)
}

/// Hands each server-sent event, a `data:` line, to `each` with its number, until the stream ends
/// or says `data: [DONE]`: what may follow that, or a connection kept open after it, is not part
/// of the answer.
async fn events(
    mut response: reqwest::Response,
    access: &Access,
    mut each: impl FnMut(&str, usize) -> Result<(), ModelError>,
) -> Result<(), ModelError> {
    let (mut pending, mut count) = (Vec::new(), 0);
    'read: loop {
        let open = more(&mut response, &mut pending, access).await?;
        if !open {
            // The last line may end without a line break; it is read like the others.
            pending.push(b'\n');
        }
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                break 'read;
            }
            count += 1;
            each(data, count)?;
        }
        if !open {
            break;
        }
    }
    if count == 0 {
        return Err(ModelError::Broken("not an event stream".into()));
    }
    Ok(())
}

/// The `n`th event, read as `T`.
fn event<T: DeserializeOwned>(data: &str, n: usize) -> Result<T, ModelError> {
    serde_json::from_str(data).map_err(|e| {
        ModelError::Broken(format!(
            "event {n}: {:?} error at column {}",
            e.classify(),
            e.column()
        ))
    })
}

/// Sends the request; any status but success becomes the provider's refusal, in its words.
async fn send(
    request: reqwest::RequestBuilder,
    access: &Access,
) -> Result<reqwest::Response, ModelError> {
    let mut response = timeout(IDLE, request.send())
        .await
        .map_err(|_| timed_out(access))?
        .map_err(|e| unreachable(access, e))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = net::retry_after(&response);
    // A provider explains an error in a few hundred bytes; more is not worth holding.
    let mut body = Vec::new();
    while body.len() < ERROR_BODY && more(&mut response, &mut body, access).await? {}
    body.truncate(ERROR_BODY);
    let body = String::from_utf8_lossy(&body);
    // OpenAI-compatible APIs explain errors in `error.message`.
    let parsed = serde_json::from_str::<Value>(&body).ok();
    let words = parsed
        .as_ref()
        .and_then(|v| v["error"]["message"].as_str().or(v["message"].as_str()));
    let code = parsed
        .as_ref()
        .and_then(|v| v["error"]["code"].as_str().or(v["error"]["type"].as_str()))
        .map(|code| without_key(code, access));
    Err(ModelError::Refused {
        status: status.as_u16(),
        retry_after,
        code,
        reply: without_key(words.unwrap_or(&body), access),
    })
}

/// The whole body of the response.
async fn read(mut response: reqwest::Response, access: &Access) -> Result<String, ModelError> {
    let mut body = Vec::new();
    while more(&mut response, &mut body, access).await? {}
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Adds the next piece of the body to `buffer`; false at its end. Gives up when the provider has
/// sent nothing for `IDLE`.
async fn more(
    response: &mut reqwest::Response,
    buffer: &mut Vec<u8>,
    access: &Access,
) -> Result<bool, ModelError> {
    let chunk = timeout(IDLE, response.chunk())
        .await
        .map_err(|_| timed_out(access))?
        .map_err(|e| unreachable(access, e))?;
    match chunk {
        Some(chunk) => {
            buffer.extend_from_slice(&chunk);
            Ok(true)
        }
        None => Ok(false),
    }
}

fn timed_out(access: &Access) -> ModelError {
    ModelError::TimedOut {
        address: access.base_url.clone(),
    }
}

fn unreachable(access: &Access, source: reqwest::Error) -> ModelError {
    if source.is_timeout() {
        timed_out(access)
    } else {
        ModelError::Unreachable {
            address: access.base_url.clone(),
            source,
        }
    }
}

/// What the provider said, in case it quotes the key back. Whole, so that what it is about is
/// found even past what the user and the log are shown.
fn without_key(text: &str, access: &Access) -> String {
    match access.api_key.as_deref() {
        None | Some("") => text.to_owned(),
        Some(key) => text.replace(key, "***"),
    }
}
