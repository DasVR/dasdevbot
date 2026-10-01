use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn daemon_logs_do_not_contain_the_session_token() {
    let dir = std::env::temp_dir().join(format!("dasdevbot-log-{}", uuid::Uuid::new_v4()));
    let web = dir.join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><body>ok</body>").unwrap();
    let data = dir.join("db.sqlite");

    let mut child = Command::new(env!("CARGO_BIN_EXE_dasdevbotd"))
        .args([
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--data",
            data.to_str().expect("utf8 path"),
            "--web",
            web.to_str().expect("utf8 path"),
            "--token",
            TOKEN,
            "--role",
            "device",
            "--provider",
            "ollama-local",
        ])
        .env_remove("OLLAMA_API_KEY")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dasdevbotd");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let _guard = ChildGuard(child);

    let (tx, rx) = std::sync::mpsc::channel();
    let stderr_thread = std::thread::spawn(move || {
        let mut logs = String::new();
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            logs.push_str(&line);
            if let Some(port) = ready_port(&line) {
                let _ = tx.send(port);
            }
            line.clear();
        }
        logs
    });
    let stdout_thread = std::thread::spawn(move || {
        let mut logs = String::new();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            logs.push_str(&line);
            line.clear();
        }
        logs
    });

    let port = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("daemon did not report a bind address");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    let start = Instant::now();
    let page = loop {
        if let Ok(response) = agent.get(&format!("http://127.0.0.1:{port}/")).call() {
            break response.into_string().unwrap();
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("daemon did not serve /");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!page.contains(TOKEN), "GET / contained the bearer");

    drop(_guard);
    let stderr_logs = stderr_thread.join().expect("stderr thread");
    let stdout_logs = stdout_thread.join().expect("stdout thread");
    assert!(
        !stderr_logs.contains(TOKEN),
        "stderr contained the session bearer"
    );
    assert!(
        !stdout_logs.contains(TOKEN),
        "stdout contained the session bearer"
    );
}

#[test]
fn allow_remote_is_refused_even_with_web() {
    let output = Command::new(env!("CARGO_BIN_EXE_dasdevbotd"))
        .args(["serve", "--web", "apps/desktop/dist", "--allow-remote"])
        .env_remove("OLLAMA_API_KEY")
        .output()
        .expect("spawn dasdevbotd");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing --allow-remote until Phase 1 or TLS"),
        "stderr was {stderr}"
    );
}

fn ready_port(line: &str) -> Option<u16> {
    let rest = line.split("bind=").nth(1)?;
    let addr = rest.split_whitespace().next()?;
    addr.rsplit_once(':')?.1.trim().parse().ok()
}
