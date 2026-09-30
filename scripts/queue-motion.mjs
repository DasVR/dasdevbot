/**
 * Full queue walk at 60fps.
 * Virtual time steps 16.667ms. Frame 0 is the Review open, the same moment
 * as the modes clip at 9.11667s. Stamps are burned after the duplicate count.
 * The clip runs until the sheet, the hold, Hello, and the undo have settled.
 */
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import net from "node:net";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));
const outDir = fileURLToPath(new URL("../docs/review/queue/", import.meta.url));
const mockSrc = "/home/ubuntu/.cursor/projects/workspace/uploads/modes-focus-away-review_d1b8.mp4";
const font = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

const STEP_MS = 16.667;
const FPS = 60;
const WIDTH = 1280;
const HEIGHT = 800;
const MOCK_START = 9.11667;
const BOOT_BUDGET_MS = 8000;
const MAX_FRAMES = 1500;

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("could not bind a local port"));
        return;
      }
      const chosen = address.port;
      server.close((err) => (err ? reject(err) : resolve(chosen)));
    });
  });
}

function drain(stream) {
  stream.on("data", () => {});
}

async function waitForHttp(url, child) {
  const deadline = Date.now() + 30_000;
  let exited = null;
  child.on("exit", (code) => {
    exited = code ?? 0;
  });
  while (Date.now() < deadline) {
    if (exited !== null) {
      throw new Error(`vite preview exited ${exited}`);
    }
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1000) });
      if (response.status < 500) {
        return;
      }
    } catch {
      // not up yet
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`preview did not respond at ${url}`);
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "inherit", "inherit"] });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} exited ${code}`));
    });
  });
}

function onceExpired(client) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("virtual time budget did not expire")), 30_000);
    client.once("Emulation.virtualTimeBudgetExpired", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function grant(client, ms, starvation) {
  const expired = onceExpired(client);
  await client.send("Emulation.setVirtualTimePolicy", {
    policy: "advance",
    budget: ms,
    maxVirtualTimeTaskStarvationCount: starvation,
  });
  await expired;
}

function approval(id, created, overrides = {}) {
  return {
    id,
    job_id: "job_m",
    agent_id: "reviewer",
    agent_name: "Reviewer",
    thread_id: "thread_reviewer",
    effect_class: "external",
    action: "post_pr_comment",
    purpose: "Leave a note on the pull request.",
    draft: "Please refresh() the branch before review.",
    evidence: { repo: "DasVR/NIL", ref: "212", event_id: "ev_abcdef", kind: "repo.push" },
    evidence_text: "repo DasVR/NIL",
    status: "pending",
    provider: "mock",
    provider_detail: "mock",
    model: "mock-review-v0",
    usage_kind: "estimated",
    input_tokens: 1200,
    output_tokens: 80,
    micro_usd: 0,
    created_at: created,
    expires_at: null,
    decided_at: null,
    decision_event_id: null,
    reason: null,
    committed: false,
    undo_until: null,
    ...overrides,
  };
}

function eventRow(id, key, hlc) {
  return {
    id,
    version: 1,
    hlc,
    source: "demo",
    kind: "approval.requested",
    thread_id: "thread_reviewer",
    idempotency_key: key,
  };
}

const T_OLD = Date.UTC(2026, 8, 30, 15, 14);
const T_MID = Date.UTC(2026, 8, 30, 15, 40);
const T_DEST = T_MID + 1000;

function baseFixture(expiresAt) {
  return {
    protocol: 1,
    role: "server",
    node: "queue-motion",
    provider: "mock",
    provider_detail: "mock",
    sync: "stub",
    agents: [
      {
        id: "reviewer",
        name: "Reviewer",
        project: "DasVR/NIL",
        persona: "Reviews the branch before anything is posted.",
        token_cap: 20000,
        tokens_spent: 1280,
        status: "blocked",
      },
    ],
    approvals: [
      approval("ap_old", T_OLD, { expires_at: expiresAt }),
      approval("ap_mid", T_MID, {
        action: "open_issue",
        expires_at: expiresAt + 15 * 60 * 1000,
        evidence: { repo: "DasVR/NIL", ref: "214", event_id: "ev_mid", kind: "repo.push" },
      }),
      approval("ap_dest", T_DEST, {
        effect_class: "destructive",
        action: "force_push",
        draft: "git push --force origin phase0",
        expires_at: expiresAt + 15 * 60 * 1000,
        evidence: { repo: "DasVR/NIL", ref: "main", event_id: "ev_dest", kind: "repo.push" },
      }),
    ],
    ledger: [],
    events: [
      eventRow("ev_abcdef", "approval-requested:ap_old", `${T_OLD}:0:node`),
      eventRow("ev_mid", "approval-requested:ap_mid", `${T_MID}:0:node`),
      eventRow("ev_dest", "approval-requested:ap_dest", `${T_DEST}:0:node`),
    ],
  };
}

function duplicateReport(hashes) {
  let duplicates = 0;
  let runs = 0;
  let isolated = 0;
  let run = 0;
  for (let index = 1; index < hashes.length; index += 1) {
    if (hashes[index] === hashes[index - 1]) {
      duplicates += 1;
      run += 1;
    } else if (run > 0) {
      runs += 1;
      if (run === 1) {
        isolated += 1;
      }
      run = 0;
    }
  }
  if (run > 0) {
    runs += 1;
    if (run === 1) {
      isolated += 1;
    }
  }
  return { frames: hashes.length, duplicates, runs, isolated };
}

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
const preview = spawn(
  process.execPath,
  [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: desktop, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, TZ: "America/New_York" } },
);
drain(preview.stdout);
drain(preview.stderr);

const decision = {
  on: false,
  undoUntil: 0,
  decidedAt: 0,
};
let clockOffset = 0;

let browser;

try {
  await waitForHttp(origin, preview);
  browser = await chromium.launch({
    channel: "chrome",
    headless: true,
    args: [
      "--disable-gpu",
      "--disable-threaded-animation",
      "--disable-threaded-scrolling",
      "--disable-background-timer-throttling",
      "--disable-renderer-backgrounding",
    ],
  });

  async function capture(reduced) {
    decision.on = false;
    const context = await browser.newContext({
      viewport: { width: WIDTH, height: HEIGHT },
      deviceScaleFactor: 1,
      timezoneId: "America/New_York",
      reducedMotion: reduced ? "reduce" : "no-preference",
    });
    const page = await context.newPage();
    const client = await context.newCDPSession(page);
    const expiresAt = Date.now() + 111_200;
    const fixture = baseFixture(expiresAt);

    await page.route("**/v1/**", async (route) => {
      const url = new URL(route.request().url());
      if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
        const body = structuredClone(fixture);
        if (decision.on) {
          const pageNow = Date.now() + clockOffset;
          const paper = pageNow >= decision.undoUntil;
          const row = body.approvals.find((item) => item.id === "ap_old");
          row.status = "approved";
          row.decided_at = decision.decidedAt;
          row.decision_event_id = "ev_0143ab";
          row.committed = paper;
          row.undo_until = paper ? null : decision.undoUntil;
        }
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(body),
        });
        return;
      }
      if (url.pathname.endsWith("/decision") && route.request().method() === "POST") {
        const pageNow = Date.now() + clockOffset + STEP_MS;
        decision.on = true;
        decision.decidedAt = pageNow;
        decision.undoUntil = pageNow + 6000;
        await route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
        return;
      }
      await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
    });

    await page.goto(origin, { waitUntil: "load" });
    await page.locator(".review-link").waitFor();
    const bootExpired = onceExpired(client);
    await client.send("Emulation.setVirtualTimePolicy", {
      policy: "pauseIfNetworkFetchesPending",
      budget: BOOT_BUDGET_MS,
      maxVirtualTimeTaskStarvationCount: 400,
    });
    await bootExpired;

    await page.evaluate(() => {
      const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
      window.__done = false;
      window.__phase = "start";
      const walk = async () => {
        window.__phase = "open";
        document.querySelector(".review-link")?.click();
        await sleep(900);
        window.__phase = "row";
        document.querySelector("[data-queue-rows] [data-queue-row]")?.click();
        await sleep(1700);
        window.__phase = "hold";
        const card = document.querySelector("article.card");
        card?.focus();
        card?.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }),
        );
        await sleep(750);
        card?.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", bubbles: true }));
        await sleep(400);
        window.__phase = "hello";
        const confirm = [...document.querySelectorAll(".hello-actions button")].find((button) =>
          button.textContent?.includes("Confirm"),
        );
        confirm?.click();
        await sleep(7600);
        window.__phase = "next";
        window.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: "ArrowDown",
            altKey: true,
            bubbles: true,
            cancelable: true,
          }),
        );
        await sleep(700);
        document.querySelector("[data-destructive]")?.scrollIntoView({ block: "center" });
        window.__phase = "rest";
        window.__done = true;
      };
      setTimeout(() => {
        void walk();
      }, 0);
    });
    await grant(client, STEP_MS, 40);

    const encoder = spawn(
      "ffmpeg",
      [
        "-y",
        "-f",
        "image2pipe",
        "-framerate",
        String(FPS),
        "-i",
        "pipe:0",
        "-r",
        String(FPS),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-tune",
        "zerolatency",
        "-pix_fmt",
        "yuv420p",
        "-an",
        reduced ? "/tmp/queue-reduced-raw.mp4" : "/tmp/queue-app-raw.mp4",
      ],
      { stdio: ["pipe", "inherit", "inherit"] },
    );

    const hashes = [];
    let previousNow = null;
    let restFrames = 0;
    let last = null;
    const transforms = [];
    const steps = [];

    for (let frame = 0; frame < MAX_FRAMES; frame += 1) {
      if (frame > 0) {
        const pageNow = await page.evaluate(() => Date.now());
        clockOffset = pageNow - Date.now();
        await grant(client, STEP_MS, 0);
      }
      const sample = await page.evaluate(() => {
        const sheet = document.querySelector("[data-review-sheet]");
        const style = sheet ? getComputedStyle(sheet) : null;
        const running = document.getAnimations().filter((anim) => anim.playState === "running").length;
        const card = document.querySelector("article.card");
        return {
          phase: window.__phase,
          done: Boolean(window.__done),
          sheet: Boolean(sheet),
          transform: style ? style.transform : "none",
          opacity: style ? style.opacity : "1",
          running,
          hello: card?.getAttribute("data-hello") ?? "off",
          paper: Boolean(card?.classList.contains("paper")),
          seen: card?.getAttribute("data-seen") ?? "",
          toast: document.querySelector("[data-toast]")?.textContent?.trim() ?? "",
          expires: document.querySelector("[data-queue-rows] .meta")?.textContent?.trim() ?? "",
          destructive: Boolean(document.querySelector("[data-destructive]")),
          now: performance.now(),
        };
      });
      if (previousNow != null) {
        const step = sample.now - previousNow;
        if (Math.abs(step - STEP_MS) > 1.5) {
          steps.push(`${frame}:${step.toFixed(2)}`);
        }
      }
      const before = sample.now;
      const shot = await client.send("Page.captureScreenshot", { format: "png" });
      const after = await page.evaluate(() => performance.now());
      if (Math.abs(after - before) > 1) {
        steps.push(`shot${frame}:${(after - before).toFixed(2)}`);
      }
      const png = Buffer.from(shot.data, "base64");
      hashes.push(createHash("sha256").update(png).digest("hex"));
      if (!encoder.stdin.write(png)) {
        await new Promise((resolve) => encoder.stdin.once("drain", resolve));
      }
      previousNow = after;
      last = sample;
      if (frame < 8 || frame % 60 === 0) {
        transforms.push(`${frame}:${sample.phase}:${sample.transform}`);
        console.error(`f${frame} ${sample.phase} sheet=${sample.sheet} hello=${sample.hello} paper=${sample.paper}`);
      }
      const settled =
        sample.done &&
        !sample.sheet &&
        sample.running === 0 &&
        sample.hello !== "open" &&
        sample.paper &&
        sample.destructive;
      if (settled) {
        restFrames += 1;
        if (restFrames === 1) {
          break;
        }
      } else {
        restFrames = 0;
      }
    }

    encoder.stdin.end();
    await new Promise((resolve, reject) => {
      encoder.on("error", reject);
      encoder.on("exit", (code) => {
        if (code === 0) {
          resolve();
          return;
        }
        reject(new Error(`ffmpeg exited ${code}`));
      });
    });
    await context.close();
    if (!last?.done) {
      throw new Error(`walk did not finish: ${JSON.stringify(last)}`);
    }
    return { hashes, transforms, last, steps };
  }

  const normal = await capture(false);
  const reduced = await capture(true);
  const normalReport = duplicateReport(normal.hashes);
  const reducedReport = duplicateReport(reduced.hashes);
  console.log("normal", JSON.stringify({ ...normalReport, steps: normal.steps, last: normal.last }));
  console.log("reduced", JSON.stringify({ ...reducedReport, steps: reduced.steps, last: reduced.last }));
  const reducedMoved = reduced.transforms.some((value) => value.includes("matrix"));
  if (reducedMoved) {
    throw new Error(`reduced motion used a transform: ${reduced.transforms.join(" | ")}`);
  }

  const stamp = `fontfile=${font}:fontsize=28:fontcolor=0x2B2723:box=1:boxcolor=0xF4F0E8@0.88:boxborderw=10:x=20:y=18`;
  const duration = (normal.hashes.length / FPS).toFixed(3);
  await run("ffmpeg", [
    "-y",
    "-ss",
    String(MOCK_START),
    "-i",
    mockSrc,
    "-i",
    "/tmp/queue-app-raw.mp4",
    "-filter_complex",
    `[0:v]scale=${WIDTH}:${HEIGHT}:flags=lanczos,tpad=stop_mode=clone:stop_duration=30,trim=duration=${duration},setpts=PTS-STARTPTS,drawtext=${stamp}:text='mock f%{n} %{eif\\:n*1000/60\\:d}ms'[left];[1:v]drawtext=${stamp}:text='app f%{n} %{eif\\:n*1000/60\\:d}ms'[right];[left][right]hstack=inputs=2,format=yuv420p`,
    "-r",
    String(FPS),
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    `${outDir}queue-vs-mock.mp4`,
  ]);
  await run("ffmpeg", [
    "-y",
    "-i",
    "/tmp/queue-reduced-raw.mp4",
    "-vf",
    `drawtext=${stamp}:text='reduced f%{n} %{eif\\:n*1000/60\\:d}ms',format=yuv420p`,
    "-r",
    String(FPS),
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    `${outDir}queue-reduced.mp4`,
  ]);
  console.log("queue-motion: pass");
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
