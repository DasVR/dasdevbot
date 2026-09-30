use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use dasdevbot_core::{authorize_secret_name, parse_role, SecretRoleError};
use dasdevbotd::{
    audit_dev_env, audit_key_path, open_provider, parse_sha256_list, plan_secret_set,
    load_role_file, prompt_secret_from_tty, read_piped_secret, serve,
    session_token_path,
    url_exposes_bearer, CommandKind, CompletionRequest, Config, Error, KeyringHandle,
    ProviderError, ProviderKind, ProviderSettings, SecretHandle, SecretSource, Store,
};

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
            "smoke-model" => smoke_model(rest),
            "secret" => secret_command(rest),
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

#[derive(Debug)]
struct Flags {
    bind: String,
    data: PathBuf,
    web: Option<PathBuf>,
    role: String,
    url: String,
    repo: String,
    reference: String,
    token: Option<String>,
    provider: ProviderKind,
    model: Option<String>,
    dev_env_secrets: bool,
    claude_home: Option<PathBuf>,
    claude_sha256: Vec<[u8; 32]>,
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
    let mut provider = ProviderKind::Ollama;
    let mut model = None;
    let mut dev_env_secrets = false;
    let mut claude_home = None;
    let mut claude_sha256 = Vec::new();
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
            "--provider" => {
                let value = value()?;
                provider = ProviderKind::parse(&value).map_err(Error::BadRequest)?;
            }
            "--model" => model = Some(value()?),
            "--dev-env-secrets" => dev_env_secrets = true,
            "--claude-home" => claude_home = Some(PathBuf::from(value()?)),
            "--claude-sha256" => {
                claude_sha256 = parse_sha256_list(&value()?).map_err(Error::BadRequest)?;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(Error::BadRequest(format!("unknown argument {other}"))),
        }
    }
    if !matches!(
        role.as_str(),
        "device" | "server" | "display" | "leader" | "worker" | "executor"
    ) {
        return Err(Error::BadRequest(
            "role must be device, server, display, leader, worker, or executor".into(),
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
        provider,
        model,
        dev_env_secrets,
        claude_home,
        claude_sha256,
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
        flags.provider.as_str()
    );
    let dev_env = flags.dev_env_secrets;
    let role = flags.role.clone();
    let provider = open_provider(&ProviderSettings {
        kind: flags.provider,
        model: flags.model.clone(),
        dev_env_secrets: flags.dev_env_secrets,
        command: CommandKind::Serve,
        role: flags.role.clone(),
        claude_home: flags.claude_home.clone(),
        claude_sha256: flags.claude_sha256.clone(),
    })?;
    let app = dasdevbotd::build_and_worker(
        Config {
            data: flags.data,
            web_root: flags.web,
            role: flags.role,
            token: explicit,
        },
        provider,
    )?;
    if dev_env {
        audit_dev_env(&app, &role)?;
    }
    serve(app, &flags.bind)
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

fn smoke_model(args: Vec<String>) -> Result<(), Error> {
    let flags = flags(args)?;
    if flags.dev_env_secrets {
        eprintln!("audit secret.dev_env role={}", flags.role);
    }
    let provider = match open_provider(&ProviderSettings {
        kind: flags.provider,
        model: flags.model,
        dev_env_secrets: flags.dev_env_secrets,
        command: CommandKind::SmokeModel,
        role: flags.role,
        claude_home: flags.claude_home,
        claude_sha256: flags.claude_sha256,
    }) {
        Ok(provider) => provider,
        Err(ProviderError::Unavailable(message)) => {
            eprintln!("dasdevbotd smoke-model skipped: {message}");
            return Ok(());
        }
        Err(err) => return Err(Error::Provider(err)),
    };
    let started = Instant::now();
    match provider.complete(
        &CompletionRequest {
            model: String::new(),
            system: String::new(),
            user: "Reply with the single word ok.".into(),
            max_tokens: 16,
        },
        &mut |_| Ok(()),
    ) {
        Ok(completion) => {
            println!("latency_ms={}", started.elapsed().as_millis());
            match (
                completion.usage.input_tokens,
                completion.usage.output_tokens,
            ) {
                (Some(input), Some(output)) => {
                    println!("input_tokens={input}");
                    println!("output_tokens={output}");
                }
                _ => println!("tokens unavailable"),
            }
            Ok(())
        }
        Err(ProviderError::Unavailable(message)) => {
            eprintln!("dasdevbotd smoke-model skipped: {message}");
            Ok(())
        }
        Err(err) => Err(Error::Provider(err)),
    }
}

fn secret_command(args: Vec<String>) -> Result<(), Error> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("set") => secret_set(args.collect()),
        Some(other) => Err(Error::BadRequest(format!(
            "unknown secret command {other}; try `secret set <name>`"
        ))),
        None => Err(Error::BadRequest(
            "secret needs a subcommand; try `secret set <name>`".into(),
        )),
    }
}

fn secret_set(args: Vec<String>) -> Result<(), Error> {
    let (data, rest) = split_data_flag(args)?;
    let plan = plan_secret_set(&rest).map_err(|err| Error::BadRequest(err.to_string()))?;
    authorize_cli_secret(&data, &plan.name)?;
    let secret = match plan.source {
        SecretSource::Tty => {
            prompt_secret_from_tty("secret: ").map_err(|err| Error::BadRequest(err.to_string()))?
        }
        SecretSource::Stdin => {
            read_piped_secret().map_err(|err| Error::BadRequest(err.to_string()))?
        }
    };
    let tail = dasdevbotd::secrets::last4(&secret);
    record_secret_audit(&data, &plan.name, tail)?;
    KeyringHandle
        .set(&plan.name, &secret)
        .map_err(|err| Error::BadRequest(err.to_string()))?;
    eprintln!("stored secret {}", plan.name);
    Ok(())
}

fn split_data_flag(args: Vec<String>) -> Result<(PathBuf, Vec<String>), Error> {
    let mut data = PathBuf::from("data/dasdevbot.sqlite");
    let mut rest = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--data" {
            let value = iter
                .next()
                .ok_or_else(|| Error::BadRequest("--data needs a value".into()))?;
            data = PathBuf::from(value);
        } else {
            rest.push(arg);
        }
    }
    Ok((data, rest))
}

fn authorize_cli_secret(data: &Path, name: &str) -> Result<(), Error> {
    let _ = data;
    let role_text = load_role_file()?;
    let Some(role) = parse_role(role_text.trim()) else {
        return Err(Error::Forbidden("daemon role is not configured".into()));
    };
    authorize_secret_name(role, name).map_err(|err| match err {
        SecretRoleError::NotAllowed => {
            Error::Forbidden("secret name is not on this role's allowlist".into())
        }
        SecretRoleError::ExecutorOnly => {
            Error::Forbidden("only the executor may store a GitHub credential".into())
        }
    })
}

fn record_secret_audit(data: &Path, name: &str, last4: String) -> Result<(), Error> {
    let mut store = Store::open(data)?;
    let seed = if audit_key_path(data).exists() {
        let text = fs::read_to_string(audit_key_path(data))?;
        dasdevbotd::signature_seed(text.trim())?
    } else {
        return Err(Error::Forbidden(
            "audit key is missing; start the daemon before storing a secret".into(),
        ));
    };
    let payload = serde_json::json!({"name": name, "last4": last4}).to_string();
    dasdevbotd::append_audit(&mut store, "secret.set", &payload, &seed)?;
    Ok(())
}

fn emit(args: Vec<String>) -> Result<(), Error> {
    let flags = flags(args)?;
    let token = emit_token(&flags)?;
    if url_exposes_bearer(&flags.url, &token) {
        return Err(Error::BadRequest(
            "bearer token must be sent only in the Authorization header".into(),
        ));
    }
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
dasdevbotd — phase 1

Usage:
  dasdevbotd serve [--bind 127.0.0.1:8787] [--data data/dasdevbot.sqlite]
                  [--web apps/desktop/dist] [--token TOKEN]
                  [--role server|leader|worker|executor|device|display]
                  [--provider ollama|ollama-local|claude-cli] [--model NAME]
                  [--claude-home PATH] [--claude-sha256 HEX] [--dev-env-secrets]
  dasdevbotd smoke-model --provider ollama|ollama-local|claude-cli [--model NAME]
                         [--claude-home PATH] [--claude-sha256 HEX]
  dasdevbotd secret set <name> [--stdin]
  dasdevbotd emit [--url http://127.0.0.1:8787] [--token TOKEN]
                  [--repo DasVR/NIL] [--ref phase0]

Mutating routes require the per-launch bearer in the Authorization header.
serve writes it next to the database as <data>.token (mode 0600). The daemon
does not put it in HTML, does not return it from the API, and does not log it.
A request that puts it in the URL or query string is rejected. The token must
be at least 32 bytes. --allow-remote is refused until Phase 1 or TLS, including
together with --web. The bind stays on loopback.

The default provider is Ollama Cloud at https://ollama.com. Store the key with
`secret set ollama` (no-echo TTY, or `--stdin` from a pipe). `--dev-env-secrets`
reads OLLAMA_API_KEY and is refused on the server role. `--model` is optional.
ollama-local talks only to 127.0.0.1:11434. claude-cli requires --claude-home.
That directory is the CLI's HOME, and <claude-home>/claude-config is its
CLAUDE_CONFIG_DIR. `--claude-sha256` is the hex digest of the native ELF.
`serve` on the server role refuses to start without it. On Ubuntu, log in once
as the service user with CLAUDE_CONFIG_DIR set to that dir. See deploy/ubuntu.
The mock provider is for tests.

The role is `serve --role` or the fixed file /etc/dasdevbot/role
(C:\\ProgramData\\dasdevbot\\role on Windows). A file next to --data is
ignored. That file must not be owned by the caller and must not be
group- or world-writable; otherwise the daemon refuses it. serve mints the
bearer when --token and DASDEVBOT_TOKEN are absent. HTTP cannot approve a
card. Decisions are Tauri IPC only. External cards need a Windows Hello
signature. Internal cards keep the Phase 1 approval-key path. See
docs/shell-ipc.md.

serve binds an iroh endpoint when the binary is built with the p2p feature
(on by default). Build with --no-default-features to leave iroh out.
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use dasdevbotd::role_path;

    #[test]
    fn allow_remote_is_refused() {
        let err = flags(vec!["--allow-remote".into()]).unwrap_err();
        assert!(err.to_string().contains("allow-remote"), "{err}");
        let err = ensure_loopback("0.0.0.0:8787").unwrap_err();
        assert!(err.to_string().contains("non-loopback"), "{err}");
    }

    #[test]
    fn a_role_file_next_to_data_cannot_authorize_github() {
        let dir = std::env::temp_dir().join(format!("dasdevbot-spoof-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let data = dir.join("x.sqlite");
        let spoof = role_path(&data);
        fs::write(&spoof, "executor").unwrap();
        assert!(spoof.ends_with("x.sqlite.role"), "{}", spoof.display());
        assert_ne!(spoof, dasdevbotd::configured_role_path());
        let literal = std::path::PathBuf::from("/tmp/x.sqlite.role");
        if spoof != literal {
            fs::write(&literal, "executor").unwrap();
        }
        let result = authorize_cli_secret(&data, "github");
        match dasdevbotd::load_role_file() {
            Err(expected) => {
                let err = result.unwrap_err();
                assert_eq!(err.to_string(), expected.to_string(), "{err}");
                assert!(
                    err.to_string().contains("not configured")
                        || err.to_string().contains("root-owned"),
                    "{err}"
                );
            }
            Ok(role) => {
                let parsed = parse_role(role.trim()).expect("fixed role parses");
                let allowed = authorize_secret_name(parsed, "github");
                assert_eq!(result.is_ok(), allowed.is_ok());
            }
        }
        let _ = fs::remove_file(literal);
    }
}
