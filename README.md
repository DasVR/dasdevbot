# dasdevbot

A clean-room, local-first runtime for AI teammates. An agent is a row — persona, memory namespace, budget — woken into a shared daemon only when an event arrives. Idle teammates cost nothing because they are not processes.

This branch is **phase 0**: the spike that has to work before the daemon language is treated as settled. It is Rust. `serve` binds a local iroh endpoint. Nothing here is copied from openbot.

## Phase 0 choices

- **IPC is loopback HTTP, JSON, protocol version 1** (`crates/proto`). The daemon binds `127.0.0.1` and blocks in `accept`, so idle means a parked thread rather than a poll loop. The same messages can move to a Tauri shell or to iroh later without a new vocabulary. A browser can speak HTTP; a named pipe cannot, and this spike's client is a browser.
- **The client is Svelte 5 + Vite**, served by the daemon from `apps/desktop/dist`. `apps/desktop/src-tauri` is the Tauri 2 scaffold and is not built here.
- **`crates/sync` links iroh 1.3 behind the `p2p` feature, which is on by default.** `serve` binds an endpoint with relays and port mapping off and prints the node id. It does not dial anyone. `cargo build --release --no-default-features -p dasdevbotd` leaves iroh out. Both sizes are in `BENCHMARKS.md`.
- **One shared worker** blocks on a channel. Inserting a job sends a wake. There is no timer and no per-agent thread.
- **Provider.** `serve` calls Ollama Cloud at the pinned host `https://ollama.com` (`/api/chat`). The key lives in the OS keychain (`dasdevbotd secret set ollama`), not in the environment, unless `--dev-env-secrets` is passed. That flag is refused on the server role. `ollama-local` is optional and talks only to `127.0.0.1:11434`. `claude-cli` runs the pinned Claude Code CLI with tools disabled. Pass `--claude-home` so the CLI's HOME is that directory and not the operator's home. On Ubuntu the `dasdevbot` service user holds only the Claude login; log in once with `sudo -u dasdevbot -H claude` (see `deploy/ubuntu`). Optional `--claude-sha256` refuses a binary whose digest differs from the one recorded at install. The mock provider is for tests. The health payload and the ledger line name which provider ran.
- **Approving does not post.** The decision is an event. The external effect is not executed.
- **License** is FSL-1.1-ALv2. See [License](#license).

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
Store an Ollama Cloud key with `dasdevbotd secret set ollama` before `serve`. `smoke-model --provider ollama` skips when that key is missing. `--dev-env-secrets` reads `OLLAMA_API_KEY` for a local check and is refused when the role is `server`.

The daemon still serves `apps/desktop/dist` on <http://127.0.0.1:8787> when that build exists. That page has no bearer, so mutating actions from it are rejected. `cd apps/desktop && npm run dev` is the Vite dev server on port 5173. It proxies `/v1` to the daemon.

## What the spike proves

1. Append-only SQLite event log with hybrid-logical-clock stamps and idempotency keys.
2. A SQLite job queue: claim, heartbeat, complete, and reclaim after the lease expires.
3. One provider call behind a trait. The mock provider covers tests. `serve` uses Ollama Cloud.
4. An IPC round trip: health, emit, snapshot, decision.
5. Agents are rows. Reviewer does no work until `repo.push` matches a stored rule.

Measurements are in `BENCHMARKS.md`. They are from a real run of `scripts/bench.py`, not estimates.

## Out of this spike

Pairing, replication over the bound endpoint, MCP, sandboxing, embeddings, Pocket, schedules, and a built Tauri shell.

## License

dasdevbot is licensed under the [Functional Source License, Version 1.1, ALv2 Future License (FSL-1.1-ALv2)](LICENSE). Copyright 2026 Arriq Al-Raee (d/b/a Das).

- **Source-available, not open source.** You may use, copy, modify, and redistribute it for any Permitted Purpose.
- **Permitted Purpose excludes competing uses.** A Competing Use means making the software available to others in a commercial product or service that substitutes for it, or that offers the same or substantially similar functionality. Internal use, non-commercial education and research, and professional services for a licensee are permitted. The [LICENSE](LICENSE) text governs.
- **Each version becomes Apache-2.0 two years after release.** Every version is irrevocably licensed under the Apache License, Version 2.0 from the second anniversary of the date it was made available.
- **Contributions require the CLA.** Before a contribution can be merged, you must agree to the [Contributor License Agreement](CLA.md). For now you do that with a comment on your pull request.
- **Trademarks.** The license grants no rights to the "dasdevbot" or "Das" names or logos. See [TRADEMARKS.md](TRADEMARKS.md).
