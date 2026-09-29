# Phase 0 measurements

These numbers were measured on this machine. They are not estimates.

| | |
|---|---|
| Host | `Linux cursor 6.12.94+ #1 SMP PREEMPT_DYNAMIC Thu Sep 24 16:04:37 UTC 2026 x86_64` |
| CPU | Intel(R) Xeon(R) Processor, 4 cores |
| Memory | 15 GiB total, about 4.7 GiB available when the release build ran |
| Rust | `rustc 1.98.1 (48a229cea 2026-09-01)` via `rust-toolchain.toml` |
| Provider during the RSS event | **mock**. `XAI_API_KEY` was unset. The draft was not from xAI. ureq and rustls are still linked. |
| iroh | not linked (`crates/sync` is a stub) |

## Profile

From the workspace `Cargo.toml`:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = false
```

`strip = false` keeps symbols in the cargo output. The stripped size below is a separate copy made with `strip --strip-unneeded`.

## Commands

Release build, timed with `time.perf_counter` around `cargo build --release -p dasdevbotd` (first release build, dependencies included):

```text
elapsed_sec 31.034
Finished `release` profile [optimized] target(s) in 30.99s
```

A later `touch crates/daemon/src/main.rs` and the same release build, no source change:

```text
incremental_elapsed_sec 8.723
Finished `release` profile [optimized] target(s) in 8.68s
```

Size, cold start, and RSS:

```bash
python3 scripts/bench.py
```

`scripts/bench.py` copies `target/release/dasdevbotd`, runs `strip --strip-unneeded` on the copy, times seven process starts until `GET /v1/health` returns 200, then reads `/proc/<pid>/status` and `ps -o rss=` after startup, after 60 seconds idle, and after one `repo.push`.

## Binary size

| Build | Bytes | MiB (1024²) |
|---|---:|---:|
| Unstripped (`target/release/dasdevbotd`) | 5,905,648 | 5.63 |
| Stripped (`strip --strip-unneeded`) | 3,053,712 | 2.91 |

**20 MB binary target: pass.** The stripped binary is under 20 × 1024² bytes (20 MiB) and under 20 × 10⁶ bytes. The unstripped binary is also under both of those.

## Idle RSS

`/proc` `VmRSS` and `ps` RSS agreed on every sample. Values are KiB as printed by the kernel (1024-byte units).

| When | VmRSS | VmHWM | RssAnon | RssFile | ps RSS |
|---|---:|---:|---:|---:|---:|
| After startup (health returned, then 0.2 s) | 4868 | 4868 | 552 | 4316 | 4868 |
| After 60 s idle | 4868 | 4868 | 552 | 4316 | 4868 |
| After one `repo.push` (mock provider) | 5020 | 5020 | 576 | 4444 | 5020 |

4868 KiB is 4.75 MiB. 5020 KiB is 4.90 MiB.

**25 MB idle RSS target: pass.** Both the startup reading and the 60 s reading are under 25 × 1024² bytes and under 25 × 10⁶ bytes. RSS did not move during the idle minute. One mock turn raised it by 152 KiB.

## Cold start

Time from process start until `GET /v1/health` returns 200. Seven runs, milliseconds:

```text
20.176
6.214
6.148
5.904
5.936
6.193
11.153
median_ms 6.193
```

The median is 6.193 ms. The design's device-daemon budget was ≤ 150 ms. This spike is under that, on this machine, without iroh.

## What this does not prove

The design's phase 0 gate asked to show the RSS budget with SQLite **and iroh** linked. iroh is not in this binary. The unused room is large (stripped binary about 17 MiB under the cap, idle RSS about 20 MiB under the cap), but that is headroom, not a measurement of iroh.
