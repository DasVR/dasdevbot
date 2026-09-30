use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

const DEFAULT_MODEL: &str = "grok-4.6";
const DEFAULT_BASE: &str = "https://api.x.ai/v1";

/// Tokens reserved before a provider call. A turn whose agent cannot cover this does not call.
pub const RESERVE_TOKENS: u64 = 256;

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub provider: String,
    pub usage_kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub micro_usd: i64,
    pub note: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProviderError(pub String);

pub trait LlmProvider: Send + Sync {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError>;
    fn id(&self) -> &'static str;
    fn detail(&self) -> String;
}

pub struct MockProvider {
    pub calls: AtomicUsize,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for MockProvider {
    fn complete(&self, _req: &CompletionRequest) -> Result<Completion, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = mock_draft();
        // Hold-scene line from 2a-approval-hold: mock · reviewer-small · 2,418 in / 212 out · $0.000431
        Ok(Completion {
            text,
            model: "reviewer-small".into(),
            provider: "mock".into(),
            usage_kind: "estimated".into(),
            input_tokens: 2_418,
            output_tokens: 212,
            micro_usd: 431,
            note: "mock provider; XAI_API_KEY is unset; the hold scene uses the look mock's labeled line"
                .into(),
        })
    }

    fn id(&self) -> &'static str {
        "mock"
    }

    fn detail(&self) -> String {
        "mock provider because XAI_API_KEY is unset".into()
    }
}

fn mock_draft() -> String {
    "If refresh() rejects on a 401, the handoff lock is never released. \
Wrap it in try/finally so the next session can take the lock."
        .into()
}

pub struct XaiProvider {
    api_key: String,
    model: String,
    base: String,
    agent: ureq::Agent,
}

impl XaiProvider {
    pub fn from_env(api_key: String) -> Self {
        let model = std::env::var("XAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let base = std::env::var("XAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.into());
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(90))
            .build();
        Self {
            api_key,
            model,
            base,
            agent,
        }
    }
}

impl LlmProvider for XaiProvider {
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        let url = format!("{}/chat/completions", self.base.trim_end_matches('/'));
        let body = chat_body(&self.model, req);
        let response = self
            .agent
            .post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|err| ProviderError(format!("xAI request failed: {err}")))?;
        let parsed: Value = response
            .into_json()
            .map_err(|err| ProviderError(format!("xAI response was not JSON: {err}")))?;
        let text = parsed["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(ProviderError("xAI returned an empty completion".into()));
        }
        let input_tokens = parsed["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        let output_tokens = parsed["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        let (micro_usd, note) = price_xai(&self.model, input_tokens, output_tokens);
        Ok(Completion {
            text,
            model: self.model.clone(),
            provider: "xai".into(),
            usage_kind: "provider".into(),
            input_tokens,
            output_tokens,
            micro_usd,
            note,
        })
    }

    fn id(&self) -> &'static str {
        "xai"
    }

    fn detail(&self) -> String {
        format!("xAI chat completions model {}", self.model)
    }
}

/// One real xAI completion. Returns `Ok(false)` when `XAI_API_KEY` is unset.
/// Does not substitute a mock response.
pub fn smoke_xai() -> Result<bool, ProviderError> {
    let key = match std::env::var("XAI_API_KEY") {
        Ok(key) if !key.trim().is_empty() => key,
        _ => {
            eprintln!("dasdevbotd smoke-xai: skipped, XAI_API_KEY is not set");
            return Ok(false);
        }
    };
    let provider = XaiProvider::from_env(key);
    let started = std::time::Instant::now();
    let completion = provider.complete(&CompletionRequest {
        model: provider.model.clone(),
        system: "Reply with the single word pong.".into(),
        user: "ping".into(),
        max_tokens: 16,
    })?;
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "provider={} model={} latency_ms={latency_ms:.1} input_tokens={} output_tokens={}",
        completion.provider, completion.model, completion.input_tokens, completion.output_tokens
    );
    println!("{}", completion.text);
    Ok(true)
}

pub fn from_env() -> Box<dyn LlmProvider> {
    match std::env::var("XAI_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Box::new(XaiProvider::from_env(key)),
        _ => Box::new(MockProvider::new()),
    }
}

pub fn chat_body(model: &str, req: &CompletionRequest) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": req.system},
            {"role": "user", "content": req.user}
        ],
        "max_tokens": req.max_tokens,
        "stream": false
    })
}

/// Published grok-4.6 rates under 200k prompt tokens: $2 / 1M input, $6 / 1M output.
/// One input token is 2 micro-USD. Other models are left unpriced (0, explained in the note).
pub fn price_xai(model: &str, input_tokens: u64, output_tokens: u64) -> (i64, String) {
    if model == "grok-4.6" && input_tokens < 200_000 {
        let micro = (input_tokens as i64)
            .saturating_mul(2)
            .saturating_add((output_tokens as i64).saturating_mul(6));
        (
            micro,
            "usage from xAI; micro_usd uses published grok-4.6 rates for prompts under 200k tokens"
                .into(),
        )
    } else {
        (
            0,
            format!("usage from xAI; model {model} is not priced in phase 0"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_body_is_a_chat_completions_request() {
        let req = CompletionRequest {
            model: "ignored".into(),
            system: "persona".into(),
            user: "event".into(),
            max_tokens: 128,
        };
        let body = chat_body("grok-4.6", &req);
        assert_eq!(body["model"], "grok-4.6");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "event");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn mock_provider_uses_the_hold_scene_line() {
        let provider = MockProvider::new();
        let done = provider
            .complete(&CompletionRequest {
                model: "x".into(),
                system: "aaaa".into(),
                user: "bbbbbbbb".into(),
                max_tokens: 32,
            })
            .unwrap();
        assert_eq!(done.provider, "mock");
        assert_eq!(done.model, "reviewer-small");
        assert_eq!(done.usage_kind, "estimated");
        assert_eq!(done.input_tokens, 2_418);
        assert_eq!(done.output_tokens, 212);
        assert_eq!(done.micro_usd, 431);
        assert!(done.text.contains("refresh()"));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn grok_4_6_price_matches_the_published_per_token_rate() {
        let (micro, _) = price_xai("grok-4.6", 1_000, 500);
        assert_eq!(micro, 1_000 * 2 + 500 * 6);
    }
}
