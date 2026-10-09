# dasdevbot threat model (Phase 1)

Status: draft for Security Director review. Scope: the desktop app (Tauri shell plus webview), the local daemon `dasdevbotd`, and its calls to model providers, in the Phase 1 build. Rulings are logged in [`DECISIONS.md`](../DECISIONS.md) (D-SD = Security Director). Details live in [`docs/phase1.md`](phase1.md) and [`docs/shell-ipc.md`](shell-ipc.md).

## Assets

| Asset | Where it lives | Why it matters |
|---|---|---|
| Per-launch session bearer | `<data>.token` beside the daemon database (mode 0600; owner-only DACL on Windows) | Authorizes every mutating daemon call and every shell-socket line |
| Approval decisions and the event log | Daemon database; decisions signed and hash-chained | An approval lets an agent's effect run. A forged or replayed one is the main harm |
| Audit key and audit tip | `<data>.audit-key`, `<data>.audit-tip` | Sign the audit chain and pin its tip (and the Windows Hello key fingerprint) |
| Per-window secrets | `<data>.window-<label>` | Let the daemon tell which window (card, main, settings) sent a socket line |
| Provider credentials | OS keychain, by handle only; a CLI provider keeps its own login store | Spend and account access with model providers |

## Trust boundaries

1. **Web page ↔ Tauri shell.** Page script is untrusted. It reaches the shell only through Tauri commands, each scoped to named windows by capability files. The page never holds the bearer.
2. **Tauri shell ↔ daemon.** Two channels:
   - Loopback HTTP on `127.0.0.1:8787`, used for the snapshot and the dev demo events.
   - A same-user local socket (Unix socket / named pipe), used for decisions and secrets.
3. **Main window ↔ card window.** Only the `card` window can sign or undo a decision. The main window can only ask to show it.
4. **Daemon ↔ model providers.**
   - Provider endpoints are pinned at compile time.
   - Credentials are used by handle and never placed in env, prompts, logs or the repo.
   - A CLI provider runs pinned and hash-checked, with tools disabled.
   - Model output is data: it never writes handover text, URLs or decisions (D-SD-003).

## In scope / out of scope

**In scope:**
- Hostile page content: XSS, a compromised dependency in the webview, a malicious site in the user's browser.
- Another OS user, or another process under a different account, on the same machine. That includes one that takes port 8787, or creates the daemon's named pipe, while the daemon is down.
- Network attackers. The daemon binds loopback only; `--allow-remote` is refused.
- Model output and teammate input trying to cause an effect without an approval.

**Out of scope:**
- **Processes running as the same OS user.** They can read the per-launch token file and the per-window secrets. Same-user processes are outside the threat model (Security Director, 2026-10-08; D-SD-012).
- A compromised OS or administrator.
- Physical access to an unlocked session.

## Current controls

- **Bearer never in JS (#36 H1, #46).** There is no command, init script, global or `<meta>` that exposes it. The shell attaches it in Rust. In dev, the vite proxy attaches it server-side, and only for same-origin requests (`Sec-Fetch-Site: same-origin`).
- **Proof of daemon (#36 M2, #46).**
  - Before sending the bearer, the shell sends a random challenge on a request that carries no bearer. The daemon answers with a keyed proof derived from the bearer, over the challenge, the request line, the status and the exact body.
  - The shell checks that the bundled daemon is still running, then sends the bearer with a fresh challenge.
  - Unproven responses are never parsed.
- **Fail-closed spawn (#36 M2).** A demo daemon that failed to start, or has exited, is a sticky error shown in the app. There is no fallback to whatever else listens on 8787.
- **Job Object (#36 M3).** On Windows the demo daemon child is in a Job Object with kill-on-close, so it dies with the app, including on a crash. If it can't be contained, it is killed and the start fails.
- **Decisions are IPC only (C10, D-SD-001).**
  - Decisions are signed card-window Tauri IPC over the same-user socket, with constant-time token checks and per-window secrets.
  - HTTP `POST /v1/approvals/{id}/decision` and `/undo` return 403.
  - Mutating HTTP calls need the bearer in the `Authorization` header only (a bearer in the URL is rejected), a loopback `Host`, and an allowed `Origin`.
- **Approval gate.**
  - Approving or undoing a card needs user verification (`verify_user`). In production this is Windows Hello; other targets refuse, so no decision can be signed without it.
  - The Hello prompt is shown in front of the card.
  - A stop during Hello wins, and a Hello result that arrives later is discarded (D-SD-007).
  - Nothing above read auto-approves (`Policy::phase1`).
  - Kernel floor (policy): routing never approves spending, messages to non-users, credential use, destructive actions or offensive tools. Enforced in code today: nothing auto-approves, every approval needs the card plus Windows Hello, destructive effects are denied, and the external tier is off. A test that enforces all five at the routing layer is tracked in #55.
- **CSP.** One policy with no `*` source. It is byte-identical in the shell config and in daemon responses, and enforced by a test.
- **External tier off.** `EXTERNAL_TIER_ENABLED = false`: external effects are hard-denied in Phase 1 until the Windows Hello hardware test and SD sign-off. Destructive effects are denied by policy (C1).
- **Dev-only demo triggers.** The scripted force-push (`FORCED_DEMO_ALLOWED`) exists only in debug builds. It is pinned by a test and never widened to the demo installer.
- **Stops.** A stop kills the whole process tree, and nothing produced after the stop becomes a result (D-SD-009). Stops need no signature but come only from authenticated trusted surfaces (D-SD-010).

## Known gaps

| Gap | Tracking | Gates |
|---|---|---|
| The bearer still crosses the loopback wire. A port takeover between the probe and the bearer request (TOCTOU) would see it | #49: per-request MAC auth mode | Any build beyond Arriq's machine; any fixed `--token` use |
| **Cross-user named-pipe squatting (in scope).** The pipe name can be derived from the data path. While the daemon is down, another OS user can create the pipe first. The client doesn't verify the server, and it sends the bearer and window secret on every line. The pipe path also skips the bundled-daemon `ready()` check that HTTP has | #54: verify the server PID or user SID before writing, and add `ready()` | Any build beyond Arriq's machine |
| The daemon child runs briefly before it is put in the Job Object | #50: create the child suspended or directly in the job | Real providers that spawn children outside the demo |
| The internal-tier approval key is in the same user's keyring, so its signature proves only "same uid" | `docs/phase1.md`; revisit in Phase 2 | Phase 2 |
| A decision signed just before a crash can commit on the next start without being shown again | #36 (undo window) | Pre-release |
| A Windows CI test timeout is under investigation; no merge on a rerun pass | #52 | Every merge |

## Related rulings

- [D-SD-001](../DECISIONS.md): CSP and signed-IPC Highs closed; decisions are IPC only.
- [D-SD-002](../DECISIONS.md): #36 H1, M2 and M3 gate G1.
- [D-SD-003](../DECISIONS.md): handover text and URLs come from fixed templates; masked key entry.
- [D-SD-007](../DECISIONS.md): a stop wins over the undo window and over Windows Hello.
- [D-SD-009](../DECISIONS.md) and [D-SD-010](../DECISIONS.md): process-tree kill and stop authority.
- [D-SD-012](../DECISIONS.md): #46 cleared for G1; residuals #49 and #50; same-user processes out of scope.
