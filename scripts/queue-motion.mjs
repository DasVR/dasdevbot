/**
 * 60fps side-by-side of the Review sheet.
 * Left: the Review-sheet moment in the modes clip, starting on the open.
 * Right: this app, advanced by Chrome virtual time at 16.667ms per frame.
 * A second file is the app under prefers-reduced-motion.
 * Duplicate frames are counted on decoded pixels, before the timestamp overlay.
 */
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
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
/** Sheet enter is 520ms. 30 steps stay inside that enter (30 × 16.667ms ≈ 500ms). */
const NORMAL_FRAMES = 30;
/** Reduced enter is 160ms. 8 steps stay inside it. */
const REDUCED_FRAMES = 8;
/** Larger than the clip, so a screenshot cannot catch the document timeline up to this budget. */
const BOOT_BUDGET_MS = 8000;

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
    const timer = setTimeout(() => reject(new Error("virtual time budget did not expire")), 20_000);
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

function approval(id, created) {
  const now = Date.now();
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
    evidence: { repo: "DasVR/NIL", ref: "212", event_id: "ev_m", kind: "repo.push" },
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
    expires_at: now + 600_000,
    decided_at: null,
    decision_event_id: null,
    reason: null,
    committed: false,
    undo_until: null,
  };
}

async function consecutiveDuplicateFrames(dir, prefix, count) {
  const raw = await new Promise((resolve, reject) => {
    const child = spawn(
      "ffmpeg",
      [
        "-v",
        "error",
        "-start_number",
        "1",
        "-i",
        `${dir}/${prefix}%04d.png`,
        "-frames:v",
        String(count),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    const chunks = [];
    const errors = [];
    child.stdout.on("data", (chunk) => chunks.push(chunk));
    child.stderr.on("data", (chunk) => errors.push(chunk));
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code !== 0) {
        reject(new Error(Buffer.concat(errors).toString() || `ffmpeg exited ${code}`));
        return;
      }
      resolve(Buffer.concat(chunks));
    });
  });
  const frameBytes = WIDTH * HEIGHT * 3;
  if (raw.length !== frameBytes * count) {
    throw new Error(`decoded ${raw.length} bytes, expected ${frameBytes * count}`);
  }
  let duplicates = 0;
  let previous = null;
  for (let index = 0; index < count; index += 1) {
    const hash = createHash("sha256")
      .update(raw.subarray(index * frameBytes, (index + 1) * frameBytes))
      .digest("hex");
    if (hash === previous) {
      duplicates += 1;
    }
    previous = hash;
  }
  return duplicates;
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

const created = [Date.UTC(2026, 8, 30, 15, 14), Date.UTC(2026, 8, 30, 15, 40), Date.UTC(2026, 8, 30, 15, 52)];
const fixture = {
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
  approvals: ["ap_a", "ap_b", "ap_c"].map((id, index) => approval(id, created[index])),
  ledger: [],
  events: ["ap_a", "ap_b", "ap_c"].map((id, index) => ({
    id: `ev_${id}`,
    version: 1,
    hlc: `${created[index]}:0:node`,
    source: "demo",
    kind: "approval.requested",
    thread_id: "thread_reviewer",
    idempotency_key: `approval-requested:${id}`,
  })),
};

let browser;
const mockDir = "/tmp/queue-motion-mock";
const appDir = "/tmp/queue-motion-app";
const reducedDir = "/tmp/queue-motion-reduced";

try {
  await waitForHttp(origin, preview);
  await rm(mockDir, { recursive: true, force: true });
  await rm(appDir, { recursive: true, force: true });
  await rm(reducedDir, { recursive: true, force: true });
  await mkdir(mockDir, { recursive: true });
  await mkdir(outDir, { recursive: true });

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

  async function capture(dir, reduced, frames) {
    await mkdir(dir, { recursive: true });
    const context = await browser.newContext({
      viewport: { width: WIDTH, height: HEIGHT },
      deviceScaleFactor: 1,
      timezoneId: "America/New_York",
      reducedMotion: reduced ? "reduce" : "no-preference",
    });
    const page = await context.newPage();
    const client = await context.newCDPSession(page);
    await page.route("**/v1/**", async (route) => {
      const url = new URL(route.request().url());
      if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(fixture),
        });
        return;
      }
      await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
    });

    // Virtual time installed before navigation deadlocks: the document fetch
    // pauses the clock, and the clock never reaches the load event.
    await page.goto(origin, { waitUntil: "load" });
    await page.locator(".review-link").waitFor();
    const bootExpired = onceExpired(client);
    await client.send("Emulation.setVirtualTimePolicy", {
      policy: "pauseIfNetworkFetchesPending",
      budget: BOOT_BUDGET_MS,
      maxVirtualTimeTaskStarvationCount: 400,
    });
    await bootExpired;

    const armed = await page.evaluate(() => {
      const link = document.querySelector(".review-link");
      if (!link) {
        return false;
      }
      setTimeout(() => {
        link.click();
      }, 0);
      return true;
    });
    if (!armed) {
      throw new Error("review link was not ready when virtual time paused");
    }
    // The click is a timer task. Svelte starts the sheet transition in the microtask
    // after that task, and the 0ms lead animation finishes on the same slice.
    await grant(client, STEP_MS, 40);

    async function readSheet() {
      return page.evaluate(() => {
        const sheet = document.querySelector("[data-review-sheet]");
        const anim = sheet?.getAnimations().find((item) => item.playState !== "idle") ?? null;
        const style = sheet ? getComputedStyle(sheet) : null;
        return {
          open: Boolean(sheet),
          ct: anim ? anim.currentTime : null,
          transform: style ? style.transform : "missing",
          opacity: style ? style.opacity : "missing",
          now: performance.now(),
        };
      });
    }

    let sample = await readSheet();
    if (!sample.open) {
      await grant(client, STEP_MS, 40);
      sample = await readSheet();
    }
    if (!sample.open || sample.ct == null) {
      throw new Error(`sheet animation did not start: ${JSON.stringify(sample)}`);
    }
    if (sample.ct < 1) {
      await page.evaluate(() => {
        const now = document.timeline.currentTime;
        for (const anim of document.getAnimations()) {
          if (anim.playState === "running" && anim.startTime == null) {
            anim.startTime = now;
          }
        }
      });
      await grant(client, STEP_MS, 0);
      sample = await readSheet();
    }

    const samples = [sample.transform];
    let previousCt = null;
    for (let frame = 0; frame < frames; frame += 1) {
      if (frame > 0) {
        await grant(client, STEP_MS, 0);
      }
      const before = await readSheet();
      if (previousCt != null) {
        const step = before.ct - previousCt;
        if (Math.abs(step - STEP_MS) > 1) {
          throw new Error(`frame ${frame} advanced ${step}ms, expected ${STEP_MS}ms`);
        }
      }
      const shot = await client.send("Page.captureScreenshot", { format: "png" });
      const after = await readSheet();
      const jump = after.ct - before.ct;
      if (Math.abs(jump) > 1) {
        throw new Error(`screenshot moved the sheet ${jump}ms at frame ${frame}`);
      }
      if (frame === 6 || frame === Math.min(18, frames - 1)) {
        samples.push(before.transform);
      }
      const name = String(frame + 1).padStart(4, "0");
      await writeFile(`${dir}/a${name}.png`, Buffer.from(shot.data, "base64"));
      previousCt = after.ct;
      if (frame === 0) {
        samples.unshift(`t0=${before.ct.toFixed(2)}ms`);
      }
    }
    await context.close();
    return samples;
  }

  const normalSamples = await capture(appDir, false, NORMAL_FRAMES);
  const reducedSamples = await capture(reducedDir, true, REDUCED_FRAMES);
  console.log("normal", normalSamples.join(" | "));
  console.log("reduced", reducedSamples.join(" | "));
  const reducedTransforms = reducedSamples.filter((value) => value !== "missing" && !value.startsWith("t0="));
  if (reducedTransforms.some((value) => value !== "none")) {
    throw new Error(`reduced motion used a transform: ${reducedTransforms.join(" | ")}`);
  }
  const normalTransforms = normalSamples.filter((value) => value.startsWith("matrix"));
  if (!normalTransforms.some((value) => value !== "none")) {
    throw new Error(`sheet did not travel: ${normalSamples.join(" | ")}`);
  }

  await run("ffmpeg", [
    "-y",
    "-i",
    mockSrc,
    "-ss",
    String(MOCK_START),
    "-frames:v",
    String(NORMAL_FRAMES),
    "-vf",
    `scale=${WIDTH}:${HEIGHT}:flags=lanczos`,
    `${mockDir}/m%04d.png`,
  ]);

  const mockDuplicates = await consecutiveDuplicateFrames(mockDir, "m", NORMAL_FRAMES);
  const appDuplicates = await consecutiveDuplicateFrames(appDir, "a", NORMAL_FRAMES);
  const reducedDuplicates = await consecutiveDuplicateFrames(reducedDir, "a", REDUCED_FRAMES);
  console.log(`duplicate frames queue-vs-mock mock: ${mockDuplicates}`);
  console.log(`duplicate frames queue-vs-mock app: ${appDuplicates}`);
  console.log(`duplicate frames queue-reduced: ${reducedDuplicates}`);
  if (mockDuplicates + appDuplicates + reducedDuplicates > 0) {
    throw new Error(
      `duplicate frames mock=${mockDuplicates} app=${appDuplicates} reduced=${reducedDuplicates}`,
    );
  }

  const stamp = `fontfile=${font}:fontsize=28:fontcolor=0x2B2723:box=1:boxcolor=0xF4F0E8@0.88:boxborderw=10:x=20:y=18`;
  await run("ffmpeg", [
    "-y",
    "-framerate",
    String(FPS),
    "-start_number",
    "1",
    "-i",
    `${mockDir}/m%04d.png`,
    "-framerate",
    String(FPS),
    "-start_number",
    "1",
    "-i",
    `${appDir}/a%04d.png`,
    "-filter_complex",
    `[0:v]drawtext=${stamp}:text='mock  %{eif\\:n*1000/60\\:d}ms'[left];[1:v]drawtext=${stamp}:text='app  %{eif\\:n*1000/60\\:d}ms'[right];[left][right]hstack=inputs=2,format=yuv420p`,
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
    "-framerate",
    String(FPS),
    "-start_number",
    "1",
    "-i",
    `${reducedDir}/a%04d.png`,
    "-vf",
    `drawtext=${stamp}:text='reduced  %{eif\\:n*1000/60\\:d}ms',format=yuv420p`,
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
  await rm(mockDir, { recursive: true, force: true });
  await rm(appDir, { recursive: true, force: true });
  await rm(reducedDir, { recursive: true, force: true });
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
