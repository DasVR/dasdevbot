# Phase 0 measurements

These numbers were measured on this machine. They are not estimates.

| | |
|---|---|
| Host | `Linux cursor 6.12.94+ #1 SMP PREEMPT_DYNAMIC Thu Sep 24 16:04:37 UTC 2026 x86_64` |
| CPU | Intel(R) Xeon(R) Processor, 4 cores |
| Memory | `MemTotal` 16398384 kB. `MemAvailable` was 4606568 kB after the round 2 benches. |
| Rust | `rustc 1.98.1 (48a229cea 2026-09-01)` via `rust-toolchain.toml` |
| Provider during RSS events | **mock**. `XAI_API_KEY` was unset. ureq and rustls are linked in both binaries. |
| iroh | 1.3.0, default crate features, behind `p2p` (on by default). `presets::Minimal`, portmapper disabled, no peers. |

## Profile

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = false
```

`strip = false` keeps symbols in the cargo output. Stripped sizes are a separate copy made with `strip --strip-unneeded`.

## Round 2: iroh linked, and the same binary without it

`serve` binds the endpoint before it listens. Health returns the node id, so the 60 s idle sample **is** idle RSS with an endpoint up. The `p2p`-off binary has no endpoint (`"endpoint_id": null`, `"sync": "off"`).

### Commands

```bash
cargo build --release -p dasdevbotd
cp target/release/dasdevbotd /tmp/dasdevbotd-p2p
cargo bloat --release -p dasdevbotd --crates -n 30
cargo build --release --no-default-features -p dasdevbotd
cp target/release/dasdevbotd /tmp/dasdevbotd-off
python3 scripts/bench.py /tmp/dasdevbotd-p2p
python3 scripts/bench.py /tmp/dasdevbotd-off
```

`scripts/bench.py` copies the given binary, runs `strip --strip-unneeded` on the copy, times seven process starts until `GET /v1/health` returns 200, then reads `/proc/<pid>/status` and `ps -o rss=` after startup, after 60 seconds idle, and after one `repo.push`.

First `p2p` release build (dependencies included), `time.perf_counter` around cargo:

```text
elapsed_sec 99.759
Finished `release` profile [optimized] target(s) in 1m 39s
```

The `--no-default-features` release after that, three local crates relinked:

```text
elapsed_sec 10.465
Finished `release` profile [optimized] target(s) in 10.42s
```

Health captured during the RSS run:

```text
p2p  sync=iroh endpoint_id=bd7590c6d4d657c76bc86b6b829cd73bb131ab59dd38fdca82f418e822fe020c
off  sync=off  endpoint_id=null
```

### Numbers

MiB is 1024². RSS KiB values are what `/proc` and `ps` printed; they agreed on every sample.

| | Stripped bytes | Stripped MiB | Idle RSS after 60 s | Idle RSS with endpoint up | Cold start median |
|---|---:|---:|---:|---:|---:|
| Earlier kernel (no iroh) | 3,053,712 | 2.91 | 4868 KiB (4.75 MiB) | not measured | 6.193 ms |
| This tree, `p2p` off | 3,055,984 | 2.91 | 4880 KiB (4.77 MiB) | no endpoint | 11.760 ms |
| This tree, `p2p` on | 7,109,416 | 6.78 | 9176 KiB (8.96 MiB) | 9176 KiB (8.96 MiB) | 11.606 ms |

Deltas of the `p2p` binary against the earlier numbers (2.91 MiB, 4.75 MiB, 6.193 ms):

| | Delta |
|---|---|
| Stripped size | +4,055,704 bytes (+3.87 MiB) |
| Idle RSS | +4308 KiB (+4.21 MiB) |
| Cold start median | +5.413 ms |

Against the new `p2p`-off binary (same source, feature off): stripped size +4,053,432 bytes (+3.86 MiB), idle RSS +4296 KiB (+4.20 MiB), cold-start median −0.154 ms. The two new medians sit inside each other's sample spread, so the bind cost is not visible above the noise on this machine. Both new medians are slower than the earlier 6.193 ms median.

Unstripped file sizes: `p2p` off 5,915,944 bytes (5.64 MiB); `p2p` on 11,961,264 bytes (11.41 MiB).

**20 MB on disk: pass**, with and without iroh. 7,109,416 is under 20 × 1024² (20,971,520) and under 20 × 10⁶.

**25 MB idle: pass**, with the endpoint up. 9176 KiB is 9,396,224 bytes, under 25 × 1024² (26,214,400) and under 25 × 10⁶.

### RSS detail

| Binary | When | VmRSS | VmHWM | RssAnon | RssFile | ps RSS |
|---|---|---:|---:|---:|---:|---:|
| `p2p` on | After health, then 0.2 s (endpoint already up) | 8988 | 8988 | 1964 | 7024 | 8988 |
| `p2p` on | After 60 s idle, endpoint still up | 9176 | 9176 | 1964 | 7212 | 9176 |
| `p2p` on | After one mock `repo.push` | 9260 | 9260 | 1984 | 7276 | 9260 |
| `p2p` off | After health, then 0.2 s | 4880 | 4880 | 552 | 4328 | 4880 |
| `p2p` off | After 60 s idle | 4880 | 4880 | 552 | 4328 | 4880 |
| `p2p` off | After one mock `repo.push` | 4964 | 4964 | 572 | 4392 | 4964 |

8988 KiB is 8.78 MiB. 9260 KiB is 9.04 MiB. 4964 KiB is 4.85 MiB. The endpoint-up RSS grew 188 KiB across the idle minute (file mappings, not anonymous).

### Cold start samples

Milliseconds, process start until health 200. Seven runs. Median is the middle sample.

`p2p` on:

```text
25.020
11.606
11.370
11.139
16.489
11.968
11.385
median_ms 11.606
```

`p2p` off:

```text
19.188
11.886
5.999
6.141
11.760
12.097
11.393
median_ms 11.760
```

### Which crate dominates

`cargo bloat --release -p dasdevbotd --crates -n 30` on the `p2p` release binary (symbols present, file 11.4 MiB, `.text` 4.5 MiB). cargo-bloat says these figures are guesswork.

| % of file | % of .text | Size | Crate |
|---:|---:|---:|---|
| 6.2% | 15.6% | 722.5 KiB | [Unknown] |
| 4.8% | 12.1% | 559.6 KiB | netlink_packet_route |
| 4.2% | 10.5% | 488.8 KiB | std |
| 3.2% | 8.0% | 370.2 KiB | iroh |
| 2.6% | 6.6% | 305.0 KiB | rustls |
| 2.1% | 5.3% | 247.1 KiB | noq_proto |
| 1.5% | 3.9% | 179.1 KiB | ring |
| 1.5% | 3.8% | 174.3 KiB | reqwest |
| 1.0% | 2.5% | 116.3 KiB | dasdevbotd |
| 0.8% | 2.0% | 90.9 KiB | portmapper |

The largest named crate is `netlink_packet_route` (559.6 KiB of `.text`). It is pulled in by iroh's netwatch/netdev stack, not by the daemon. The `iroh` crate itself is next among the p2p crates, at 370.2 KiB. `noq_proto`, `portmapper`, `iroh_relay`, `igd_next`, `n0_dns_resolver`, `noq`, `netwatch`, `netdev`, and `iroh_dns` are the rest of that neighborhood in the top 30.

iroh does not blow either budget. Stripped size is 13.2 MiB under 20 MiB. Idle RSS with the endpoint up is 16.0 MiB under 25 MiB.

## Round 1 baseline (kept)

Measured before iroh was linked. Same profile, same script, binary was `target/release/dasdevbotd` with `crates/sync` as a stub. Do not treat the round 2 rows as a re-run of these samples.

Release build:

```text
elapsed_sec 31.034
Finished `release` profile [optimized] target(s) in 30.99s
```

Touch rebuild of `crates/daemon/src/main.rs`, no source change:

```text
incremental_elapsed_sec 8.723
Finished `release` profile [optimized] target(s) in 8.68s
```

| Build | Bytes | MiB (1024²) |
|---|---:|---:|
| Unstripped | 5,905,648 | 5.63 |
| Stripped (`strip --strip-unneeded`) | 3,053,712 | 2.91 |

| When | VmRSS | VmHWM | RssAnon | RssFile | ps RSS |
|---|---:|---:|---:|---:|---:|
| After startup (health returned, then 0.2 s) | 4868 | 4868 | 552 | 4316 | 4868 |
| After 60 s idle | 4868 | 4868 | 552 | 4316 | 4868 |
| After one `repo.push` (mock provider) | 5020 | 5020 | 576 | 4444 | 5020 |

4868 KiB is 4.75 MiB. 5020 KiB is 4.90 MiB. RSS did not move during that idle minute.

Cold start, seven runs, milliseconds:

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

## smoke-xai

`XAI_API_KEY` was unset in this environment. The command was:

```bash
./target/release/dasdevbotd smoke-xai
```

It printed `dasdevbotd smoke-xai: skipped, XAI_API_KEY is not set` and exited 0. No completion was invented, so this document has no latency or token counts for xAI.
