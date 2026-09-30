# dasdevbot

A clean-room, local-first runtime for AI teammates. An agent is a row — persona, memory namespace, budget — woken into a shared daemon only when an event arrives. Idle teammates cost nothing because they are not processes.

This branch is **phase 0**: the spike that has to work before the daemon language is treated as settled. It is Rust. `serve` binds a local iroh endpoint. Nothing here is copied from openbot.

## Phase 0 choices

- **IPC is loopback HTTP, JSON, protocol version 1** (`crates/proto`). The daemon binds `127.0.0.1` and blocks in `accept`, so idle means a parked thread rather than a poll loop. The same messages can move to a Tauri shell or to iroh later without a new vocabulary. A browser can speak HTTP; a named pipe cannot, and this spike's client is a browser.
- **The client is Svelte 5 + Vite**, served by the daemon from `apps/desktop/dist`. `apps/desktop/src-tauri` is the Tauri 2 scaffold and is not built here.
- **`crates/sync` links iroh 1.3 behind the `p2p` feature, which is on by default.** `serve` binds an endpoint with relays and port mapping off and prints the node id. It does not dial anyone. `cargo build --release --no-default-features -p dasdevbotd` leaves iroh out. Both sizes are in `BENCHMARKS.md`.
- **One shared worker** blocks on a channel. Inserting a job sends a wake. There is no timer and no per-agent thread.
- **Provider.** `XAI_API_KEY` selects xAI chat completions (`https://api.x.ai/v1/chat/completions`, model `grok-4.6` unless `XAI_MODEL` is set). If the key is missing, a **mock provider** writes the draft. Mock output is prefixed with `[mock provider]`. Token counts on that path are `char/4` estimates and the charge is `$0.00`. The health payload and the ledger line name which one ran.
- **Approving does not post.** The decision is an event. The external effect is not executed.
- **License** is undecided (`UNLICENSED`). Commercial intent is still an open decision.

## Layout

```
crates/core     events, gate, budgets, leases (no I/O)
crates/proto    versioned daemon↔client messages
crates/sync     iroh endpoint (`p2p`, default on)
crates/daemon   dasdevbotd
apps/desktop    Svelte 5 client and a Tauri 2 scaffold
integrations/   placeholders only
```

## How to run

Requirements: Rust 1.98.1 (see `rust-toolchain.toml`), Node 22, and a C compiler for bundled SQLite.

```bash
cargo test --workspace
cargo build --release

cd apps/desktop
npm install
npm run build
cd ../..

./target/release/dasdevbotd serve
```

With the daemon running, start the Vite dev server and use that page for the clickable demo:

```bash
cd apps/desktop && npm run dev
```

Open <http://127.0.0.1:5173>. Click **Simulate repo.push**. Reviewer wakes, drafts a review, and the approval card appears. **Approve** or **Deny**. The event log gains `approval.decided`. Nothing is sent to GitHub or anywhere else. The dev server injects the bearer into the page it serves. `npm run build` does not.

From another shell, against a running daemon:

```bash
./target/release/dasdevbotd emit --repo DasVR/NIL --ref phase0
```

Useful flags: `--bind`, `--data` (default `data/dasdevbot.sqlite`), `--web`, `--role server|device|display`, `--token`. The role is recorded; phase 0 behavior does not change with it. The bind stays on loopback. `--allow-remote` is refused until Phase 1 or TLS, including together with `--web`.

`serve` mints a bearer of at least 32 random bytes, writes it to `<data>.token` (mode 0600), and does not put it in any HTML it serves. The API does not return it, and the daemon does not log it. Send it only in the `Authorization` header. A request that puts the bearer in the URL or query string is rejected. `emit` reads the token file, or `--token`. The Tauri shell reads that file and passes the bearer to the webview through an init script (`window.__DASDEVBOT_TOKEN`) and the `session_token` IPC command.

Set `XAI_API_KEY` before `serve` to call xAI. Leave it unset to stay on the mock provider. Do not commit the key. The xAI base URL is the compile-time constant `https://api.x.ai/v1`. The process environment cannot redirect it.

One real call, with no mock fallback:

```bash
XAI_API_KEY=... ./target/release/dasdevbotd smoke-xai
```

If `XAI_API_KEY` is unset, that command prints `dasdevbotd smoke-xai: skipped, XAI_API_KEY is not set` and exits 0. It does not invent a completion. `XAI_MODEL` overrides the model (default `grok-4.6`).

The daemon still serves `apps/desktop/dist` on <http://127.0.0.1:8787> when that build exists. That page has no bearer, so mutating actions from it are rejected. `cd apps/desktop && npm run dev` is the Vite dev server on port 5173. It proxies `/v1` to the daemon.

## What the spike proves

1. Append-only SQLite event log with hybrid-logical-clock stamps and idempotency keys.
2. A SQLite job queue: claim, heartbeat, complete, and reclaim after the lease expires.
3. One provider call behind a trait, with the mock fallback above.
4. An IPC round trip: health, emit, snapshot, decision.
5. Agents are rows. Reviewer does no work until `repo.push` matches a stored rule.

Measurements are in `BENCHMARKS.md`. They are from a real run of `scripts/bench.py`, not estimates.

## Out of this spike

Pairing, replication over the bound endpoint, MCP, sandboxing, embeddings, Pocket, schedules, and a built Tauri shell.
