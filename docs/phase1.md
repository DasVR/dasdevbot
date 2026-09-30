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
| `crates/daemon/src/audit_log.rs` | Append-only hash chain. The key sits beside the DB. The tip is also in `<data>.audit-tip`. |
| `crates/daemon/src/hello_key.rs` | External tier: Windows Hello public key only. Other targets deny external. |
| `crates/daemon/src/ipc.rs` | Internal cards use the approval seed. External cards require a Hello signature. |
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

`ToolUseAttempted` fails the job closed. The attempted audit payload stays empty. The blocked audit payload is `{"job_id","cli_version"}` and does not include the tool event.

## Audit chain

The signing key is `<data>.audit-key`, beside the database. Anyone who can replace the database can replace that key. There is no external witness. The latest tip hash is also written to `<data>.audit-tip`. Opening a database refuses a new genesis when that sidecar still holds a prior tip, and refuses a sidecar that does not match the log. `secret.dev_env`, grant seeding, and gate denials are rows on this chain, as are `secret.set`, `approval.decided`, `approval.undone`, `hello.enrolled`, and `hello.reset`.

The sidecar also pins the Windows Hello key: a second line `hello-pin <fingerprint>` (blake3 of the blob type and the SPKI hex). Enrollment writes `hello.enrolled` before the key row and the pin. A key whose fingerprint doesn't match the pin is refused, and verification uses the stored key only when it matches the pin. Deleting the row does not free the pin. Re-enrollment needs the `hello-reset` shell op: settings window, a successful user check, and a reason. It writes `hello.reset` before it clears the row and the pin.

`secret set` on the CLI audits only into the daemon database `/var/lib/dasdevbot/dasdevbot.sqlite`. Any other `--data` is refused, and the database and its audit key must already exist, so a throwaway database cannot satisfy the audit.

## Permission gate

Destructive work is denied in phase 1 and never asked. `repo.force_push` records a denied card and does not call the provider. The payload does not choose the tier. HTTP `POST /v1/approvals/{id}/decision` and `/undo` return 403 with a fixed body. Mutating HTTP requires a bearer token, loopback `Host`, and no `Access-Control-Allow-Origin: *`. `--allow-remote` is refused. Voice, CLI, and banners cannot decide.

**External is hard-denied in Phase 1.** `dasdevbot_core::EXTERNAL_TIER_ENABLED` is `false`. `gate::decide` returns `Deny` for external under every policy, `authorize_decision` returns `DecisionDeny::External`, and `prepare` refuses external cards, whatever signature or verifier is present. The external tier is re-enabled only after all three of these pass: H1 (RSA-2048 PKCS#1 v1.5 / SHA-256 Hello verify), H2 (key pinning and audited enrollment and reset), and the hardware Hello test in `docs/hello-hardware-test.md`. Re-enabling it is a separate, reviewed change.

Internal tiers (`read`, `write_local`, and the other non-external classes) keep the Phase 1 Ed25519 path. `sign_decision` signs with the `approval-key` seed and the daemon verifies that same signature. **That signature proves nothing beyond "a process running as the daemon's uid asked".** The seed is in the same user's keyring, and the daemon both signs and verifies. This is accepted for Phase 1 and must be revisited in Phase 2. The signature is kept as a record that binds the decision to the card, the nonce and the fencing token. It is not an authentication factor.

The external path, once enabled, is different: the shell signs with a non-exportable RSA-2048 Hello key from `KeyCredentialManager`, and the daemon stores only the pinned public key and verifies it with CNG. The shell shows one named consent prompt (decision, action, target), and `RequestSignAsync` is the key-bound check. The daemon doesn't add its own Hello prompt. On every other target the external tier stays denied, including when the seed is present. The shell protocol is `docs/shell-ipc.md`.

`set_secret` is a capability of `main` and `settings` only. The role is `serve --role` or `/etc/dasdevbot/role` (`C:\ProgramData\dasdevbot\role` on Windows). A file next to `--data` is not a role. The fixed file must be root-owned. It is refused when it is not owned by root, when the caller owns it (so a root caller always refuses it), or when group or other can write it. **On Windows, `serve --role` is the only role source.** The ownership check isn't implemented there, so the fixed file is always refused. **On Windows, secrets go in only through the Tauri settings window.** `secret set` on the CLI is refused. `serve` refuses to start when neither source gives a role. The audit row is written before the keyring write, and a failed audit does not store the secret.

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
