//! Ollama Cloud is the default provider. The host is the compile-time constant
//! `https://ollama.com`. `ollama-local` is optional and talks only to `127.0.0.1:11434`.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::{
    log_provider, Busy, Completion, CompletionRequest, Headroom, ProviderError, QuotaSignal,
    RetryCost, UsageReport,
};
use crate::secrets::{scrub, Secret};

pub const OLLAMA_CLOUD_ORIGIN: &str = "https://ollama.com";
pub const OLLAMA_CHAT_PATH: &str = "/api/chat";
pub const OLLAMA_CLOUD_MODEL: &str = "gemma4:31b";
pub const OLLAMA_LOCAL_ORIGIN: &str = "http://127.0.0.1:11434";
pub const OLLAMA_LOCAL_MODEL: &str = "llama3.2";
pub const OLLAMA_LOCAL_PORT: u16 = 11434;

/// First wait after a busy response.
pub const OLLAMA_BUSY_INITIAL_MS: u64 = 2_000;
/// Upper bound on a single wait. Doubling never exceeds this.
pub const OLLAMA_BUSY_MAX_MS: u64 = 60_000;
/// Total chat attempts, including the first. The next failure is terminal.
pub const OLLAMA_BUSY_MAX_ATTEMPTS: u32 = 6;
/// Budget tokens recorded for each busy retry. This is daemon accounting, not a model token count.
pub const OLLAMA_BUSY_RETRY_BUDGET_TOKENS: u64 = 1;

/// Delay before another attempt after `failed_attempt` failures. `None` means the cap was hit.
/// `Retry-After` does not change this schedule.
pub fn busy_backoff_after(failed_attempt: u32) -> Option<u64> {
    if failed_attempt == 0 || failed_attempt >= OLLAMA_BUSY_MAX_ATTEMPTS {
        return None;
    }
    Some(capped_busy_delay(failed_attempt - 1))
}

pub fn capped_busy_delay(step: u32) -> u64 {
    let shift = step.min(16);
    OLLAMA_BUSY_INITIAL_MS
        .saturating_mul(1_u64 << shift)
        .min(OLLAMA_BUSY_MAX_MS)
}

const CLOUD_QUOTA_ABSENT: &str = "Ollama Cloud chat responses include per-call token counts. GET /api/usage is the only account signal, and only a remaining monthly-credit fraction is accepted. No session or weekly limit is read.";

pub const OLLAMA_USAGE_URL: &str = "https://ollama.com/api/usage";

pub fn cloud_chat_url() -> String {
    format!("{OLLAMA_CLOUD_ORIGIN}{OLLAMA_CHAT_PATH}")
}

pub fn local_chat_url() -> &'static str {
    "http://127.0.0.1:11434/api/chat"
}

pub struct OllamaCloud {
    secret: Secret,
    model: Option<String>,
    agent: ureq::Agent,
    headroom: Mutex<Headroom>,
}

impl OllamaCloud {
    pub fn new(secret: Secret, model: Option<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .redirects(0)
            .build();
        Self {
            secret,
            model,
            agent,
            headroom: Mutex::new(Headroom::unknown()),
        }
    }

    pub fn complete(
        &self,
        req: &CompletionRequest,
        charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        let model = choose_model(self.model.as_deref(), &req.model, OLLAMA_CLOUD_MODEL);
        let mut completion = run_busy_retries(
            || self.post_chat(&model, req),
            charge,
            |backoff_ms| std::thread::sleep(Duration::from_millis(backoff_ms)),
        )?;
        self.observe_usage(&mut completion);
        Ok(completion)
    }

    fn post_chat(&self, model: &str, req: &CompletionRequest) -> Result<Completion, ProviderError> {
        let body = chat_body(model, req);
        let rendered = body.to_string();
        if !self.secret.expose().is_empty() && rendered.contains(self.secret.expose()) {
            return Err(ProviderError::Failed(
                "ollama request body contained the api key".into(),
            ));
        }
        let response = self
            .agent
            .post(&cloud_chat_url())
            .set("Authorization", &format!("Bearer {}", self.secret.expose()))
            .set("Content-Type", "application/json")
            .send_json(body);
        match response {
            Ok(resp) => {
                let headers = header_pairs(&resp);
                let parsed: Value = resp.into_json().map_err(|err| {
                    cloud_failed(
                        &self.secret,
                        format!("ollama response was not JSON ({})", err.kind()),
                    )
                })?;
                parse_cloud_success(model, &headers, &parsed, &self.secret)
            }
            Err(ureq::Error::Status(status, resp)) => {
                let headers = header_pairs(&resp);
                let body = resp.into_string().unwrap_or_default();
                Err(cloud_status(&self.secret, status, &headers, &body))
            }
            Err(err) => Err(cloud_failed(
                &self.secret,
                format!("ollama cloud request failed ({})", err.kind()),
            )),
        }
    }

    fn observe_usage(&self, completion: &mut Completion) {
        let observed = match self
            .agent
            .get(OLLAMA_USAGE_URL)
            .set("Authorization", &format!("Bearer {}", self.secret.expose()))
            .call()
        {
            Ok(resp) => resp
                .into_json::<Value>()
                .ok()
                .as_ref()
                .and_then(parse_monthly_credit),
            Err(_) => None,
        };
        let mut headroom = self.headroom.lock().expect("headroom");
        headroom.lower(observed);
        completion.usage.headroom = headroom.clone();
        if observed.is_some() {
            log_provider("ollama usage: monthly credit signal applied");
        } else {
            log_provider("ollama usage: no monthly-credit signal");
        }
    }

    pub fn detail(&self) -> String {
        let model = self.model.as_deref().unwrap_or(OLLAMA_CLOUD_MODEL);
        format!("ollama cloud model {model}")
    }
}

pub struct OllamaLocal {
    model: Option<String>,
    agent: ureq::Agent,
}

impl OllamaLocal {
    pub fn new(model: Option<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .redirects(0)
            .build();
        Self { model, agent }
    }

    pub fn complete(
        &self,
        req: &CompletionRequest,
        _charge: &mut dyn FnMut(&RetryCost) -> Result<(), ProviderError>,
    ) -> Result<Completion, ProviderError> {
        let listeners = current_listeners()?;
        guard_local(&listeners)?;
        let model = choose_model(self.model.as_deref(), &req.model, OLLAMA_LOCAL_MODEL);
        let body = chat_body(&model, req);
        let response = self
            .agent
            .post(local_chat_url())
            .set("Content-Type", "application/json")
            .send_json(body);
        match response {
            Ok(resp) => {
                let headers = header_pairs(&resp);
                let parsed: Value = resp.into_json().map_err(|err| {
                    ProviderError::Failed(format!(
                        "ollama-local response was not JSON ({})",
                        err.kind()
                    ))
                })?;
                parse_local_success(&model, &headers, &parsed)
            }
            Err(ureq::Error::Status(status, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                Err(ProviderError::Failed(format!(
                    "ollama-local returned HTTP {status}: {}",
                    truncate(&body)
                )))
            }
            Err(err) if err.kind() == ureq::ErrorKind::ConnectionFailed => {
                Err(ProviderError::Unavailable(format!(
                    "ollama-local is not listening at {OLLAMA_LOCAL_ORIGIN}"
                )))
            }
            Err(err) => Err(ProviderError::Failed(format!(
                "ollama-local request failed ({})",
                err.kind()
            ))),
        }
    }

    pub fn detail(&self) -> String {
        let model = self.model.as_deref().unwrap_or(OLLAMA_LOCAL_MODEL);
        format!("ollama-local model {model} at {OLLAMA_LOCAL_ORIGIN}")
    }
}

pub fn chat_body(model: &str, req: &CompletionRequest) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": req.system},
            {"role": "user", "content": req.user}
        ],
        "stream": false
    })
}

fn choose_model(configured: Option<&str>, request: &str, default: &str) -> String {
    if let Some(model) = configured.filter(|model| !model.is_empty()) {
        return model.to_string();
    }
    if !request.is_empty() {
        return request.to_string();
    }
    default.to_string()
}

fn header_pairs(resp: &ureq::Response) -> Vec<(String, String)> {
    resp.headers_names()
        .into_iter()
        .filter_map(|name| resp.header(&name).map(|value| (name, value.to_string())))
        .collect()
}

/// A concurrency 429 is retryable. A credits 429, and any other 429, is a terminal limit.
pub fn cloud_status(
    secret: &Secret,
    status: u16,
    _headers: &[(String, String)],
    body: &str,
) -> ProviderError {
    let scrubbed = scrub(body, secret);
    if is_concurrency_limit(&scrubbed) && !is_credits_exhausted(&scrubbed) {
        log_provider("ollama cloud busy");
        return ProviderError::Busy(Busy {
            message: "ollama cloud is busy".into(),
            retry_costs: Vec::new(),
            terminal: false,
        });
    }
    if status == 429 || is_credits_exhausted(&scrubbed) {
        let message = if is_credits_exhausted(&scrubbed) {
            "ollama cloud credits are exhausted"
        } else {
            "ollama cloud rate limit is not concurrency"
        };
        log_provider(message);
        return ProviderError::LimitReached(super::LimitReached {
            message: message.into(),
            resets_at: None,
        });
    }
    cloud_failed(
        secret,
        format!(
            "ollama cloud returned HTTP {status}: {}",
            truncate(&scrubbed)
        ),
    )
}

fn is_concurrency_limit(body: &str) -> bool {
    body.to_ascii_lowercase()
        .contains("too many concurrent requests")
}

fn is_credits_exhausted(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("insufficient credit")
        || lower.contains("credits exhausted")
        || lower.contains("credit limit")
        || lower.contains("quota exceeded")
        || lower.contains("usage limit")
        || lower.contains("out of credits")
        || lower.contains("monthly credit")
}

/// Run chat attempts. After a non-terminal busy response, charge the retry and only then wait and call again.
pub fn run_busy_retries<Call, Charge, Wait>(
    mut call: Call,
    mut charge: Charge,
    mut wait: Wait,
) -> Result<Completion, ProviderError>
where
    Call: FnMut() -> Result<Completion, ProviderError>,
    Charge: FnMut(&RetryCost) -> Result<(), ProviderError>,
    Wait: FnMut(u64),
{
    let mut retry_costs = Vec::new();
    for attempt in 1..=OLLAMA_BUSY_MAX_ATTEMPTS {
        match call() {
            Ok(mut completion) => {
                completion.retry_costs = retry_costs;
                return Ok(completion);
            }
            Err(ProviderError::Busy(busy)) if !busy.terminal => {
                let Some(backoff_ms) = busy_backoff_after(attempt) else {
                    return Err(terminal_busy(retry_costs));
                };
                let cost = RetryCost {
                    attempt,
                    backoff_ms,
                    budget_tokens: OLLAMA_BUSY_RETRY_BUDGET_TOKENS,
                };
                charge(&cost)?;
                retry_costs.push(cost);
                log_provider(&format!(
                    "ollama cloud busy attempt={attempt} backoff_ms={backoff_ms}"
                ));
                wait(backoff_ms);
            }
            Err(err) => return Err(err),
        }
    }
    Err(terminal_busy(retry_costs))
}

fn terminal_busy(retry_costs: Vec<RetryCost>) -> ProviderError {
    log_provider("ollama cloud busy terminal");
    ProviderError::Busy(Busy {
        message: "ollama cloud is busy".into(),
        retry_costs,
        terminal: true,
    })
}

fn cloud_failed(secret: &Secret, message: impl AsRef<str>) -> ProviderError {
    let line = scrub(message.as_ref(), secret);
    log_provider(&line);
    ProviderError::Failed(line)
}

fn parse_cloud_success(
    model: &str,
    headers: &[(String, String)],
    parsed: &Value,
    secret: &Secret,
) -> Result<Completion, ProviderError> {
    let text = message_text(parsed).map_err(|err| cloud_failed(secret, err.to_string()))?;
    let usage = usage_from_body(parsed);
    let header_note = quota_header_note(headers, secret);
    let (quota, note) = match header_note {
        Some(note) => (
            QuotaSignal::Reported {
                detail: format!("{note} Token counts are the per-call metrics only."),
            },
            format!("usage from ollama cloud; {note}"),
        ),
        None => (
            QuotaSignal::Absent {
                detail: CLOUD_QUOTA_ABSENT.into(),
            },
            cloud_note(&usage),
        ),
    };
    Ok(Completion::from_usage(
        text,
        model,
        "ollama",
        usage_kind(&usage),
        0,
        note,
        UsageReport { quota, ..usage },
    ))
}

fn parse_local_success(
    model: &str,
    headers: &[(String, String)],
    parsed: &Value,
) -> Result<Completion, ProviderError> {
    let text = message_text(parsed)?;
    let usage = usage_from_body(parsed);
    let header_note = quota_header_note(headers, &Secret::new(""));
    let quota = match header_note {
        Some(note) => QuotaSignal::Reported { detail: note },
        None => QuotaSignal::Absent {
            detail: CLOUD_QUOTA_ABSENT.into(),
        },
    };
    Ok(Completion::from_usage(
        text,
        model,
        "ollama-local",
        usage_kind(&usage),
        0,
        "local ollama is not priced; account quota is not part of the loopback API",
        UsageReport { quota, ..usage },
    ))
}

fn message_text(parsed: &Value) -> Result<String, ProviderError> {
    let text = parsed["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        Err(ProviderError::Failed(
            "ollama returned an empty completion".into(),
        ))
    } else {
        Ok(text)
    }
}

fn usage_from_body(parsed: &Value) -> UsageReport {
    UsageReport {
        input_tokens: parsed["prompt_eval_count"].as_u64(),
        output_tokens: parsed["eval_count"].as_u64(),
        cached_input_tokens: parsed["prompt_eval_cached_count"].as_u64(),
        quota: QuotaSignal::Absent {
            detail: String::new(),
        },
        headroom: Headroom::unknown(),
    }
}

fn usage_kind(usage: &UsageReport) -> &'static str {
    if usage.input_tokens.is_some() || usage.output_tokens.is_some() {
        "provider"
    } else {
        "absent"
    }
}

fn cloud_note(usage: &UsageReport) -> String {
    match (usage.input_tokens, usage.output_tokens) {
        (Some(input), Some(output)) => format!(
            "usage from ollama cloud prompt_eval_count={input} eval_count={output}; local charge is 0; {CLOUD_QUOTA_ABSENT}"
        ),
        _ => format!("ollama cloud omitted token counts; {CLOUD_QUOTA_ABSENT}"),
    }
}

fn quota_header_note(headers: &[(String, String)], secret: &Secret) -> Option<String> {
    let mut parts = Vec::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == "retry-after"
            || lower.contains("ratelimit")
            || lower.contains("rate-limit")
            || lower.contains("quota")
        {
            parts.push(format!("{name}={}", scrub(value, secret)));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("rate-limit headers: {}", parts.join(", ")))
    }
}

/// The only accepted `GET /api/usage` body. A remaining monthly credit is a fraction in `0..=1`.
/// Old session and weekly shapes, extra keys, and dollar amounts are no signal.
pub fn parse_monthly_credit(value: &Value) -> Option<f64> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let credits = obj.get("monthly_credits")?.as_object()?;
    if credits.len() != 1 {
        return None;
    }
    let remaining = credits.get("remaining")?.as_f64()?;
    if remaining.is_finite() && (0.0..=1.0).contains(&remaining) {
        Some(remaining)
    } else {
        None
    }
}

fn truncate(text: &str) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index == 240 {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

pub fn guard_local(listeners: &[SocketAddr]) -> Result<(), ProviderError> {
    let lan: Vec<SocketAddr> = listeners
        .iter()
        .copied()
        .filter(|addr| addr.port() == OLLAMA_LOCAL_PORT && !is_loopback(*addr))
        .collect();
    if lan.is_empty() {
        Ok(())
    } else {
        Err(ProviderError::Failed(format!(
            "ollama-local refuses to run because Ollama is listening on a LAN address: {lan:?}"
        )))
    }
}

fn is_loopback(addr: SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

fn current_listeners() -> Result<Vec<SocketAddr>, ProviderError> {
    listeners_from("/proc/net/tcp", "/proc/net/tcp6")
}

fn listeners_from(tcp_path: &str, tcp6_path: &str) -> Result<Vec<SocketAddr>, ProviderError> {
    let tcp = fs::read_to_string(tcp_path).map_err(|err| {
        ProviderError::Failed(format!(
            "cannot verify ollama is not listening on a LAN address ({})",
            err.kind()
        ))
    })?;
    let tcp6 = fs::read_to_string(tcp6_path).unwrap_or_default();
    Ok(parse_listen_sockets(&tcp, &tcp6))
}

pub fn parse_listen_sockets(tcp: &str, tcp6: &str) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    out.extend(parse_proc_table(tcp, false));
    out.extend(parse_proc_table(tcp6, true));
    out
}

fn parse_proc_table(text: &str, v6: bool) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let _sl = fields.next();
        let Some(local) = fields.next() else {
            continue;
        };
        let _remote = fields.next();
        let Some(state) = fields.next() else {
            continue;
        };
        if !state.eq_ignore_ascii_case("0A") {
            continue;
        }
        let Some((addr_hex, port_hex)) = local.split_once(':') else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(port_hex, 16) else {
            continue;
        };
        let ip = if v6 {
            parse_v6(addr_hex).map(IpAddr::V6)
        } else {
            parse_v4(addr_hex).map(IpAddr::V4)
        };
        let Some(ip) = ip else {
            continue;
        };
        out.push(SocketAddr::new(ip, port));
    }
    out
}

fn parse_v4(hex: &str) -> Option<Ipv4Addr> {
    if hex.len() != 8 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let bytes = value.to_be_bytes();
    Some(Ipv4Addr::new(bytes[3], bytes[2], bytes[1], bytes[0]))
}

fn parse_v6(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for group in 0..4 {
        let start = group * 8;
        let word = u32::from_str_radix(&hex[start..start + 8], 16).ok()?;
        let part = word.to_be_bytes();
        bytes[group * 4] = part[3];
        bytes[group * 4 + 1] = part[2];
        bytes[group * 4 + 2] = part[1];
        bytes[group * 4 + 3] = part[0];
    }
    Some(Ipv6Addr::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{start_log, take_log, Headroom};
    use serde_json::json;

    #[test]
    fn busy_backoff_matches_the_spec_constants() {
        assert_eq!(OLLAMA_BUSY_INITIAL_MS, 2_000);
        assert_eq!(OLLAMA_BUSY_MAX_MS, 60_000);
        assert_eq!(OLLAMA_BUSY_MAX_ATTEMPTS, 6);
        assert_eq!(busy_backoff_after(0), None);
        assert_eq!(busy_backoff_after(1), Some(2_000));
        assert_eq!(busy_backoff_after(2), Some(4_000));
        assert_eq!(busy_backoff_after(3), Some(8_000));
        assert_eq!(busy_backoff_after(4), Some(16_000));
        assert_eq!(busy_backoff_after(5), Some(32_000));
        assert_eq!(busy_backoff_after(6), None);
        assert_eq!(capped_busy_delay(5), 60_000);
    }

    #[test]
    fn concurrency_is_busy_and_the_body_is_not_logged() {
        let secret = Secret::new("SENTINEL_KEY");
        start_log();
        let err = cloud_status(
            &secret,
            429,
            &[("retry-after".into(), "9".into())],
            "too many concurrent requests SENTINEL_KEY",
        );
        let logs = take_log();
        match err {
            ProviderError::Busy(busy) => {
                assert!(!busy.terminal);
                assert!(!busy.to_string().contains("SENTINEL_KEY"));
            }
            other => panic!("expected busy, got {other}"),
        }
        assert!(logs.iter().all(|line| !line.contains("SENTINEL_KEY")));
        assert!(logs.iter().any(|line| line == "ollama cloud busy"));
        let phrase = cloud_status(&secret, 500, &[], "Too Many Concurrent Requests");
        assert!(matches!(phrase, ProviderError::Busy(_)));
        let other = cloud_status(&secret, 401, &[], "invalid credentials");
        assert!(matches!(other, ProviderError::Failed(_)));
    }

    #[test]
    fn chat_body_does_not_carry_a_key_and_usage_is_the_pinned_get() {
        let req = CompletionRequest {
            model: "ignored".into(),
            system: "persona".into(),
            user: "hello".into(),
            max_tokens: 16,
        };
        let body = chat_body("gemma4:31b", &req);
        assert_eq!(body["stream"], false);
        assert_eq!(cloud_chat_url(), "https://ollama.com/api/chat");
        assert_eq!(OLLAMA_USAGE_URL, "https://ollama.com/api/usage");
        assert!(!body.to_string().contains("SENTINEL_KEY"));
    }

    #[test]
    fn monthly_credit_accepts_only_the_strict_fraction() {
        assert_eq!(
            parse_monthly_credit(&json!({"monthly_credits": {"remaining": 0.4}})),
            Some(0.4)
        );
        assert_eq!(
            parse_monthly_credit(&json!({"monthly_credits": {"remaining": 0}})),
            Some(0.0)
        );
        assert_eq!(
            parse_monthly_credit(&json!({"monthly_credits": {"remaining": 1}})),
            Some(1.0)
        );
        assert_eq!(
            parse_monthly_credit(&json!({"monthly_credits": {"remaining": 1.5}})),
            None
        );
        assert_eq!(
            parse_monthly_credit(
                &json!({"limits": {"session": {"usage": 0.2}, "weekly": {"usage": 0.1}}})
            ),
            None
        );
        assert_eq!(
            parse_monthly_credit(&json!({"monthly_credits": {"remaining": 0.4, "used": 0.1}})),
            None
        );
        assert_eq!(
            parse_monthly_credit(&json!({"monthly_credits": {"remaining": 0.4}, "extra": true})),
            None
        );
    }

    #[test]
    fn headroom_only_lowers() {
        let mut headroom = Headroom::unknown();
        headroom.lower(Some(0.9));
        assert_eq!(headroom.remaining_credit(), Some(0.9));
        headroom.lower(Some(0.9));
        assert_eq!(headroom.remaining_credit(), Some(0.9));
        headroom.lower(Some(0.4));
        assert_eq!(headroom.remaining_credit(), Some(0.4));
        headroom.lower(Some(0.9));
        assert_eq!(headroom.remaining_credit(), Some(0.4));
        headroom.lower(Some(0.2));
        assert_eq!(headroom.remaining_credit(), Some(0.2));
        headroom.lower(Some(1.5));
        headroom.lower(Some(f64::NAN));
        headroom.lower(None);
        assert_eq!(headroom.remaining_credit(), Some(0.2));
    }

    #[test]
    fn lan_listeners_are_refused_and_loopback_is_not() {
        let tcp = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:2CAA 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1 1 0000000000000000 100 0 0 10 0
   1: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 2 1 0000000000000000 100 0 0 10 0
   2: 0501A8C0:2CAA 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 3 1 0000000000000000 100 0 0 10 0
   3: 00000000:2CAA 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 6 1 0000000000000000 100 0 0 10 0
";
        let tcp6 = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:2CAA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:2CAA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 5 1 0000000000000000 100 0 0 10 0
";
        let listeners = parse_listen_sockets(tcp, tcp6);
        assert!(listeners.contains(&"127.0.0.1:11434".parse().unwrap()));
        assert!(listeners.contains(&"[::1]:11434".parse().unwrap()));
        assert!(listeners.contains(&"0.0.0.0:80".parse().unwrap()));
        let loopback_only = parse_listen_sockets(
            "  sl  local_address rem_address   st\n   0: 0100007F:2CAA 00000000:0000 0A\n",
            "  sl  local_address remote_address st\n   0: 00000000000000000000000001000000:2CAA 00000000000000000000000000000000:0000 0A\n",
        );
        assert!(guard_local(&loopback_only).is_ok());
        let err = guard_local(&listeners).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("192.168.1.5"));
        assert!(text.contains("0.0.0.0:11434"));
        assert!(text.contains("[::]"));
        assert!(!text.contains("0.0.0.0:80"));
    }

    #[test]
    fn missing_proc_table_fails_closed() {
        let err = listeners_from("/no/such/proc/net/tcp", "/no/such/proc/net/tcp6").unwrap_err();
        assert!(err.to_string().contains("LAN"));
    }

    #[test]
    fn credits_429_is_terminal_and_concurrency_429_is_busy() {
        let secret = Secret::new("SENTINEL_KEY");
        let credits = cloud_status(
            &secret,
            429,
            &[],
            r#"{"error":"monthly credit limit reached SENTINEL_KEY"}"#,
        );
        match credits {
            ProviderError::LimitReached(limit) => {
                assert!(limit.message.contains("credits"));
                assert!(!limit.to_string().contains("SENTINEL_KEY"));
            }
            other => panic!("expected a credit limit, got {other}"),
        }
        let bare = cloud_status(&secret, 429, &[], "");
        assert!(matches!(bare, ProviderError::LimitReached(_)));
        let busy = cloud_status(&secret, 429, &[], "too many concurrent requests");
        assert!(matches!(busy, ProviderError::Busy(_)));
    }

    #[test]
    fn a_retry_is_charged_before_the_next_call() {
        let trace = std::cell::RefCell::new(Vec::new());
        let mut calls = 0;
        let completion = run_busy_retries(
            || {
                calls += 1;
                trace.borrow_mut().push(format!("call{calls}"));
                if calls < 3 {
                    Err(ProviderError::Busy(Busy {
                        message: "ollama cloud is busy".into(),
                        retry_costs: Vec::new(),
                        terminal: false,
                    }))
                } else {
                    Ok(sample_completion())
                }
            },
            |cost| {
                trace.borrow_mut().push(format!("charge{}", cost.attempt));
                Ok(())
            },
            |backoff_ms| trace.borrow_mut().push(format!("wait{backoff_ms}")),
        )
        .unwrap();
        assert_eq!(
            trace.into_inner(),
            vec![
                "call1".to_string(),
                "charge1".to_string(),
                "wait2000".to_string(),
                "call2".to_string(),
                "charge2".to_string(),
                "wait4000".to_string(),
                "call3".to_string(),
            ]
        );
        assert_eq!(completion.retry_costs.len(), 2);

        let mut calls = 0;
        let err = run_busy_retries(
            || {
                calls += 1;
                Err(ProviderError::Busy(Busy {
                    message: "ollama cloud is busy".into(),
                    retry_costs: Vec::new(),
                    terminal: false,
                }))
            },
            |_| Err(ProviderError::Failed("budget".into())),
            |_| panic!("a refused charge must not wait"),
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(matches!(err, ProviderError::Failed(_)));
    }

    fn sample_completion() -> Completion {
        Completion {
            text: "ok".into(),
            model: OLLAMA_CLOUD_MODEL.into(),
            provider: "ollama".into(),
            usage_kind: "absent".into(),
            input_tokens: 0,
            output_tokens: 0,
            micro_usd: 0,
            note: String::new(),
            usage: crate::provider::UsageReport {
                input_tokens: None,
                output_tokens: None,
                cached_input_tokens: None,
                quota: crate::provider::QuotaSignal::Absent {
                    detail: String::new(),
                },
                headroom: Headroom::unknown(),
            },
            retry_costs: Vec::new(),
        }
    }
}
