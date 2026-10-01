#!/usr/bin/env python3
"""Measure dasdevbotd size, cold start, and idle RSS. Prints what it measured."""

import os
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("DASDEVBOTD_BIN", ROOT / "target" / "release" / "dasdevbotd"))
RUNS = 7


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_health(port: int, timeout: float = 15.0) -> str:
    url = f"http://127.0.0.1:{port}/v1/health"
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=0.2) as response:
                if response.status == 200:
                    return response.read().decode()
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            time.sleep(0.005)
    raise SystemExit(f"health check failed on {port}")


def rss_kb(pid: int) -> dict[str, str]:
    values: dict[str, str] = {}
    status = Path(f"/proc/{pid}/status").read_text()
    for key in ("VmRSS", "VmHWM", "RssAnon", "RssFile"):
        for line in status.splitlines():
            if line.startswith(key + ":"):
                values[key] = line.split()[1]
    ps = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)], text=True).strip()
    values["ps_rss"] = ps
    return values


def cold_starts() -> list[float]:
    samples: list[float] = []
    for _ in range(RUNS):
        port = free_port()
        data = Path(tempfile.mkdtemp(prefix="dasdevbot-cold-")) / "db.sqlite"
        start = time.perf_counter()
        proc = subprocess.Popen(
            [str(BIN), "serve", "--role", "server", "--bind", f"127.0.0.1:{port}", "--data", str(data)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            wait_health(port)
            samples.append((time.perf_counter() - start) * 1000)
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=2)
    return samples


def rss_scenario() -> dict[str, dict[str, str]]:
    port = free_port()
    directory = Path(tempfile.mkdtemp(prefix="dasdevbot-rss-"))
    data = directory / "db.sqlite"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--role", "server", "--bind", f"127.0.0.1:{port}", "--data", str(data)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        health = wait_health(port)
        print(f"health {health.strip()}")
        time.sleep(0.2)
        after_start = rss_kb(proc.pid)
        time.sleep(60)
        after_idle = rss_kb(proc.pid)
        body = (
            b'{"source":"bench","kind":"repo.push","payload":'
            b'{"repo":"DasVR/NIL","ref":"bench"},"idempotency_key":"bench-push-1"}'
        )
        token = Path(f"{data}.token").read_text().strip()
        request = urllib.request.Request(
            f"http://127.0.0.1:{port}/v1/events",
            data=body,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {token}",
            },
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=5) as response:
            response.read()
        time.sleep(0.5)
        after_event = rss_kb(proc.pid)
        return {"after_start": after_start, "after_60s_idle": after_idle, "after_one_event": after_event}
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=2)


def main() -> None:
    global BIN
    if len(sys.argv) > 1:
        BIN = Path(sys.argv[1])
    if not BIN.is_file():
        raise SystemExit(f"missing {BIN}; cargo build --release first")
    print(f"bin {BIN}")
    unstripped = BIN.stat().st_size
    stripped_path = Path(tempfile.mkdtemp(prefix="dasdevbot-strip-")) / "dasdevbotd"
    shutil.copy(BIN, stripped_path)
    subprocess.check_call(["strip", "--strip-unneeded", str(stripped_path)])
    stripped = stripped_path.stat().st_size
    print("machine")
    print(subprocess.check_output(["uname", "-a"], text=True).strip())
    print(subprocess.check_output(["rustc", "--version"], text=True).strip())
    cpu = next(
        (line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")),
        "unknown",
    )
    print(f"cpu {cpu}")
    print(f"nproc {os.cpu_count()}")
    print("sizes")
    print(f"unstripped_bytes {unstripped}")
    print(f"stripped_bytes {stripped}")
    print("cold_start_ms")
    samples = cold_starts()
    for sample in samples:
        print(f"{sample:.3f}")
    print(f"median_ms {statistics.median(samples):.3f}")
    print("rss_kib")
    for label, values in rss_scenario().items():
        parts = " ".join(f"{key}={value}" for key, value in values.items())
        print(f"{label} {parts}")


if __name__ == "__main__":
    main()
