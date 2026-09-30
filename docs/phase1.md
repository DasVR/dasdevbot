# Phase 1 backend

`dasdevbotd` is still one Rust binary. Phase 1 adds topology, the teammate harness, the admission ledger, and the permission gate. Ollama Cloud, local Ollama, and the Claude CLI live in `crates/daemon/src/provider/`. The harness talks to `dasdevbot_core::Provider`. Tests use the mock `LlmProvider`. Finn-specific behavior was naming only, so it was removed: the runtime is the generic harness, and nothing Finn-branded is built.

## Module map

| Module | Role |
|---|---|
| `crates/core/src/roles.rs` | `leader`, `worker`, `device`, `executor`. `server` parses as leader. Only the executor writes the ledger. Secret names are an allowlist; only the executor may store `github` or `github-app`. |
| `crates/core/src/ownership.rs` | Who owns each data class, and the device → server replica list. |
| `crates/core/src/election.rs` | SQLite leader lease. Takeover bumps the fencing token. |
| `crates/core/src/admission.rs` | 1.5× headroom check, monthly vs signal window, concurrency slots, fencing. |
| `crates/core/src/provider.rs` | Provider trait, stub, and the charged retry driver. |
| `crates/core/src/harness.rs` | Phases: wake, plan, act, request-approval, checkpoint, resume, finish. |
| `crates/core/src/decision.rs` | Card-window Tauri IPC is the only approval surface. |
| `crates/core/src/grant.rs` | Grants can only shorten expiry. Destructive grants are not issued. |
| `crates/core/src/secret.rs` | Redacted secret values. |
| `crates/core/src/gate.rs` | Phase 1 denies destructive. Taint blocks auto-approve above read. |
| `crates/daemon/src/schema.rs` | Phase 1 tables, including the fenced provider ledger. |
| `crates/daemon/src/caps.rs` | Ledger read/write. Single writer, fencing epoch, slot cap. |
| `crates/daemon/src/topology.rs` | Leader assigns pending jobs. Workers claim only their assignment. |
| `crates/daemon/src/harness.rs` | Checkpointed harness run. Busy and the retry cap pause. Tool use fails closed. |
| `crates/daemon/src/audit_log.rs` | Append-only hash chain (`prev_hash` per row). Tool-use rows have an empty payload. |
| `crates/daemon/src/ipc.rs` | `sign_decision` signs the daemon's action material. `undo_decision` and secret-window check. |
| `crates/daemon/src/secrets.rs` | OS keychain secret store. `secret set` is the operator CLI. Tauri secret entry uses the role allowlist. |
| `crates/daemon/src/ownership_store.rs` | Device chats cannot set `replicate_to_server`. Server batch is job metadata. |
| `apps/desktop` | Decisions go through Tauri IPC. Secret entry is `#settings` only. |
| `deploy/ubuntu` | Loopback systemd units. One executor writes the ledger. |

## Ownership

| Class | Owner | Device → server |
|---|---|---|
| chat, chat_message, secret, approval_decision | device | no |
| job_metadata | server | yes |
| provider_cap_ledger | server | no (not a device replica) |

Chats live in the device SQLite. The server batch ignores any flag that asks to include them.

## Ledger

Before a job starts, the estimate is covered at 1.5× (`cover_tokens`). An unknown estimate uses the teammate `conservative_max`. Zero means deny. A budget-store read or write error denies. It does not fall through to another provider.

A provider limit signal can only lower headroom. A missing or failed signal leaves the ledger number as-is. That is not a store error.

Ollama-shaped windows are monthly (`WindowKind::Monthly`): when `now` passes `reset_at_ms`, spent drops and the reset moves forward. Claude-shaped windows (`WindowKind::SignalReset`) take the reset time from a present signal. A monthly window is not replaced by a later signal reset.

Each provider also has a concurrency cap. No slot means `waiting_on_slot`. No credit means `waiting_on_quota` (or ask the owner when there is no reset). Quota is checked first.

The ledger is single-writer. `provider_caps.fencing_epoch` plus `writer_id` / `writer_until_ms` reject a stale or foreign writer. Phase 1 runs one executor. The systemd unit says not to start a second one.

The reservation is the provider's worst-case estimate (`input + max_tokens`, with `max_tokens` 512 on the turn). The check is 1.5× that estimate. A missing caps row denies. The demo path seeds a `mock` cap whose writer lease is already expired, so the executor can take it. Before every attempt, including the first and each retry, that worst case is reserved under the fencing epoch. When the call commits, spent becomes the measured input plus output and the unused hold is released. A failed attempt keeps the worst-case hold as spent. If the hold cannot be placed, the job pauses.

`RETRY_CAP` is 3 on the harness driver. Ollama Cloud retries a busy call up to 6 times. The charge callback reserves the same worst-case estimate before each of those retries. It does not reserve the 1-token accounting field. Terminal `Busy` or `Limit` pauses the job. The claim query does not pick up `paused`. A second `execute` on a paused checkpoint does not call the provider.

`ToolUseAttempted` fails the job closed and appends an audit row whose payload is empty.

## Permission gate

Destructive work is denied in phase 1 and never asked. `repo.force_push` records a denied card and does not call the provider. The payload does not choose the tier. HTTP `POST /v1/approvals/{id}/decision` and `/undo` return 403 with a fixed body. Mutating HTTP requires a bearer token, loopback `Host`, and no `Access-Control-Allow-Origin: *`. `--allow-remote` is refused. Voice, CLI, and banners cannot decide. Sign and undo are capabilities of the `card` window, and the command reads `window.label()` at runtime. `sign` is an Ed25519 signature over the daemon's action hash, nonce, and fencing token, using the `approval-key` secret. `set_secret` is a capability of `main` and `settings` only, and the role is the daemon's configured role.

The secret field is component `$state`. The draft is cleared before the invoke. It is not a Svelte store, `localStorage`, or plugin-store. After a save the UI shows the last 4 characters.

## What is stubbed

- The act step does not call tools. The driver runs the provider stub or a test script.
- Ollama Cloud, local Ollama, and the Claude CLI are in `crates/daemon/src/provider/`. Tests that must not call them use `ollama-fake`, `claude-fake`, `StubProvider`, and `MockProvider`. There is no GitHub App client.
- Secret tests use `MemorySecrets`. `KeyringHandle` is the `keyring` crate and is not exercised against a live secret service here.
- Leader election is a SQLite lease, not a cluster protocol.
- One executor instance. Split-brain fencing is the column and the CAS, not a deployed second server.
- Grants are seeded with `issue_expiry`. A row with no expiry does not count.
- No PAKE, TPM, witness log, signed updater, SQLCipher, voice capture, or session unlock.

## Ubuntu

`deploy/ubuntu/dasdevbotd.service` binds `127.0.0.1:8787` as user `dasdevbot` with data in `/var/lib/dasdevbot`. The optional worker unit shares that file and cannot write the ledger. The device unit keeps its own SQLite.
