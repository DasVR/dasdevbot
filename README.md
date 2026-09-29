# dasdevbot

A clean-room, local-first runtime for AI teammates. An agent is a row — persona, memory namespace, budget — woken into a shared daemon only when an event arrives. Idle teammates cost nothing because they are not processes.

This branch is **phase 0**: the spike that has to work before the daemon language is treated as settled. It is Rust. Sync is a stub. Nothing here is copied from openbot.

## Phase 0 choices

- **IPC is loopback HTTP, JSON, protocol version 1** (`crates/proto`). The daemon binds `127.0.0.1` and blocks in `accept`, so idle means a parked thread rather than a poll loop. The same messages can move to a Tauri shell or to iroh later without a new vocabulary. A browser can speak HTTP; a named pipe cannot, and this spike's client is a browser.
- **The client is Svelte 5 + Vite**, served by the daemon from `apps/desktop/dist`. `apps/desktop/src-tauri` is the Tauri 2 scaffold and is not built here.
- **`crates/sync` does not link iroh.** The design gate wanted iroh in the binary before trusting the RSS budget. This spike measures the kernel alone (SQLite, the queue, one provider call, loopback IPC) so a 20 MB binary is a statement about that kernel. Headroom, if any, is in `BENCHMARKS.md`.
- **One shared worker** blocks on a channel. Inserting a job sends a wake. There is no timer and no per-agent thread.
- **Provider.** `XAI_API_KEY` selects xAI chat completions (`https://api.x.ai/v1/chat/completions`, model `grok-4.6` unless `XAI_MODEL` is set). If the key is missing, a **mock provider** writes the draft. Mock output is prefixed with `[mock provider]`. Token counts on that path are `char/4` estimates and the charge is `$0.00`. The health payload and the ledger line name which one ran.
- **Approving does not post.** The decision is an event. The external effect is not executed.
- **License** is undecided (`UNLICENSED`). Commercial intent is still an open decision.

## Layout

```
crates/core     events, gate, budgets, leases (no I/O)
crates/proto    versioned daemon↔client messages
crates/sync     stub (iroh not linked)
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

Open <http://127.0.0.1:8787>. Click **Simulate repo.push**. Reviewer wakes, drafts a review, and the approval card appears. **Approve** or **Deny**. The event log gains `approval.decided`. Nothing is sent to GitHub or anywhere else.

From another shell, against a running daemon:

```bash
./target/release/dasdevbotd emit --repo DasVR/NIL --ref phase0
```

Useful flags: `--bind`, `--data` (default `data/dasdevbot.sqlite`), `--web`, `--role server|device|display`. The role is recorded; phase 0 behavior does not change with it. Non-loopback binds need `--allow-remote`.

Set `XAI_API_KEY` before `serve` to call xAI. Leave it unset to stay on the mock provider. Do not commit the key.

`cd apps/desktop && npm run dev` is the Vite dev server on port 5173. It proxies `/v1` to the daemon.

## What the spike proves

1. Append-only SQLite event log with hybrid-logical-clock stamps and idempotency keys.
2. A SQLite job queue: claim, heartbeat, complete, and reclaim after the lease expires.
3. One provider call behind a trait, with the mock fallback above.
4. An IPC round trip: health, emit, snapshot, decision.
5. Agents are rows. Reviewer does no work until `repo.push` matches a stored rule.

Measurements are in `BENCHMARKS.md`. They are from a real run of `scripts/bench.py`, not estimates.

## Out of this spike

Pairing, iroh, MCP, sandboxing, embeddings, Pocket, schedules, and a built Tauri shell.
