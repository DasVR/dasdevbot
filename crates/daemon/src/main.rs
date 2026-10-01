use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use dasdevbotd::{serve, session_token_path, url_exposes_bearer, Error};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("dasdevbotd: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Error> {
    let mut args = env::args().skip(1).peekable();
    if args.peek().is_none() {
        return serve_from(Vec::new());
    }
    let first = args.next().unwrap();
    if first == "--help" || first == "-h" {
        print_help();
        return Ok(());
    }
    let mut rest: Vec<String> = args.collect();
    if first.starts_with('-') {
        rest.insert(0, first);
        serve_from(rest)
    } else {
        match first.as_str() {
            "serve" => serve_from(rest),
            "emit" => emit(rest),
            "smoke-xai" => smoke_xai(rest),
            "help" => {
                print_help();
                Ok(())
            }
            other => Err(Error::BadRequest(format!(
                "unknown command {other}; try --help"
            ))),
        }
    }
}

struct Flags {
    bind: String,
    data: PathBuf,
    web: Option<PathBuf>,
    role: String,
    url: String,
    repo: String,
    reference: String,
    token: Option<String>,
}

fn flags(args: Vec<String>) -> Result<Flags, Error> {
    let mut bind = "127.0.0.1:8787".to_string();
    let mut data = PathBuf::from("data/dasdevbot.sqlite");
    let mut web: Option<PathBuf> = None;
    let mut web_set = false;
    let mut role = "server".to_string();
    let mut url = "http://127.0.0.1:8787".to_string();
    let mut repo = "DasVR/NIL".to_string();
    let mut reference = "phase0".to_string();
    let mut token: Option<String> = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .ok_or_else(|| Error::BadRequest(format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--bind" => bind = value()?,
            "--data" => data = PathBuf::from(value()?),
            "--web" => {
                web = Some(PathBuf::from(value()?));
                web_set = true;
            }
            "--role" => role = value()?,
            "--url" => url = value()?,
            "--repo" => repo = value()?,
            "--ref" => reference = value()?,
            "--allow-remote" => {
                return Err(Error::BadRequest(
                    "refusing --allow-remote until Phase 1 or TLS".into(),
                ));
            }
            "--token" => token = Some(value()?),
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(Error::BadRequest(format!("unknown argument {other}"))),
        }
    }
    if !matches!(role.as_str(), "device" | "server" | "display") {
        return Err(Error::BadRequest(
            "role must be device, server, or display".into(),
        ));
    }
    if !web_set {
        let default = PathBuf::from("apps/desktop/dist");
        if default.join("index.html").is_file() {
            web = Some(default);
        }
    }
    Ok(Flags {
        bind,
        data,
        web,
        role,
        url,
        repo,
        reference,
        token,
    })
}

fn serve_from(args: Vec<String>) -> Result<(), Error> {
    let flags = flags(args)?;
    ensure_loopback(&flags.bind)?;
    let explicit = explicit_token(flags.token);
    eprintln!(
        "dasdevbotd starting role={} data={} provider={}",
        flags.role,
        flags.data.display(),
        provider_label()
    );
    serve(
        dasdevbotd::build_and_worker(dasdevbotd::Config {
            data: flags.data,
            web_root: flags.web,
            role: flags.role,
            token: explicit,
        })?,
        &flags.bind,
    )
}

fn explicit_token(flag: Option<String>) -> Option<String> {
    if let Some(token) = flag.filter(|token| !token.trim().is_empty()) {
        return Some(token);
    }
    match env::var("DASDEVBOT_TOKEN") {
        Ok(token) if !token.trim().is_empty() => Some(token),
        _ => None,
    }
}

fn emit(args: Vec<String>) -> Result<(), Error> {
    let flags = flags(args)?;
    let endpoint = format!("{}/v1/events", flags.url.trim_end_matches('/'));
    let body = serde_json::json!({
        "source": "cli",
        "kind": "repo.push",
        "payload": {
            "repo": flags.repo,
            "ref": flags.reference,
            "subject": "simulated push",
            "note": "phase 0 attaches no diff"
        },
        "idempotency_key": format!("cli-{}", uuid::Uuid::new_v4())
    });
    let token = emit_token(&flags)?;
    if url_exposes_bearer(&flags.url, &token) {
        return Err(Error::BadRequest(
            "bearer token must be sent only in the Authorization header".into(),
        ));
    }
    let response = ureq::post(&endpoint)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(|err| Error::BadRequest(format!("emit failed: {err}")))?;
    let text = response
        .into_string()
        .map_err(|err| Error::BadRequest(err.to_string()))?;
    println!("{text}");
    Ok(())
}

fn smoke_xai(args: Vec<String>) -> Result<(), Error> {
    if !args.is_empty() {
        return Err(Error::BadRequest("smoke-xai takes no arguments".into()));
    }
    dasdevbotd::smoke_xai()?;
    Ok(())
}

fn emit_token(flags: &Flags) -> Result<String, Error> {
    if let Some(token) = explicit_token(flags.token.clone()) {
        return Ok(token);
    }
    let path = session_token_path(&flags.data);
    match std::fs::read_to_string(&path) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_string()),
        _ => Err(Error::BadRequest(format!(
            "missing bearer token; pass --token or set DASDEVBOT_TOKEN (looked at {})",
            path.display()
        ))),
    }
}

fn provider_label() -> &'static str {
    match env::var("XAI_API_KEY") {
        Ok(key) if !key.trim().is_empty() => "xai",
        _ => "mock (XAI_API_KEY unset)",
    }
}

fn ensure_loopback(bind: &str) -> Result<(), Error> {
    let host = bind.rsplit_once(':').map(|(host, _)| host).unwrap_or(bind);
    let host = host.trim_matches(['[', ']']);
    if host == "127.0.0.1" || host == "localhost" || host == "::1" {
        Ok(())
    } else {
        Err(Error::BadRequest("refusing a non-loopback bind".into()))
    }
}

fn print_help() {
    eprintln!(
        "\
dasdevbotd — phase 0 spike

Usage:
  dasdevbotd serve [--bind 127.0.0.1:8787] [--data data/dasdevbot.sqlite]
                  [--web apps/desktop/dist] [--role server|device|display]
                  [--token TOKEN]
  dasdevbotd emit [--url http://127.0.0.1:8787] [--token TOKEN]
                  [--repo DasVR/NIL] [--ref phase0]
  dasdevbotd smoke-xai

Mutating routes require the per-launch bearer in the Authorization header.
serve writes it next to the database as <data>.token (mode 0600). The daemon
does not put it in HTML, does not return it from the API, and does not log it.
A request that puts it in the URL or query string is rejected. The token must
be at least 32 bytes. --allow-remote is refused until Phase 1 or TLS, including
together with --web. The bind stays on loopback.

The provider is xAI chat completions when XAI_API_KEY is set.
Otherwise every draft is produced by the labeled mock provider.
smoke-xai does not use the mock: it skips when XAI_API_KEY is unset.
XAI_MODEL overrides the model (default grok-4.6).
The xAI base URL is the compile-time constant https://api.x.ai/v1.

serve binds an iroh endpoint when the binary is built with the p2p feature
(on by default). Build with --no-default-features to leave iroh out.
"
    );
}
