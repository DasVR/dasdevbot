/**
 * 60fps side-by-side of the Review sheet.
 * Left: the Review-sheet moment in the modes clip, starting on the open.
 * Right: this app, seeked on the same input through the Web Animations timeline.
 * A second file is the app under prefers-reduced-motion.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import net from "node:net";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));
const outDir = fileURLToPath(new URL("../docs/review/queue/", import.meta.url));
const mockSrc = "/home/ubuntu/.cursor/projects/workspace/uploads/modes-focus-away-review_d1b8.mp4";
const font = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

const FRAMES = 90;
const FPS = 60;
const MOCK_START = 9.11667;

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

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
const preview = spawn(
  process.execPath,
  [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: desktop, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, TZ: "America/New_York" } },
);
drain(preview.stdout);
drain(preview.stderr);

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
    model: "mock-review-v0",
    usage_kind: "estimated",
    input_tokens: 1200,
    output_tokens: 80,
    micro_usd: 0,
    created_at: created,
    expires_at: now + 111_200,
    decided_at: null,
    decision_event_id: null,
    reason: null,
    committed: false,
    undo_until: null,
  };
}

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
try {
  await waitForHttp(origin, preview);
  const mockDir = "/tmp/queue-motion-mock";
  const appDir = "/tmp/queue-motion-app";
  const reducedDir = "/tmp/queue-motion-reduced";
  await rm(mockDir, { recursive: true, force: true });
  await rm(appDir, { recursive: true, force: true });
  await rm(reducedDir, { recursive: true, force: true });
  await mkdir(mockDir, { recursive: true });
  await mkdir(outDir, { recursive: true });

  await run("ffmpeg", [
    "-y",
    "-i",
    mockSrc,
    "-ss",
    String(MOCK_START),
    "-frames:v",
    String(FRAMES),
    "-vf",
    "scale=1280:800:flags=lanczos",
    `${mockDir}/m%04d.png`,
  ]);

  browser = await chromium.launch({ channel: "chrome" });

  async function capture(dir, reduced) {
    await mkdir(dir, { recursive: true });
    const context = await browser.newContext({
      viewport: { width: 1280, height: 800 },
      deviceScaleFactor: 1,
      timezoneId: "America/New_York",
      reducedMotion: reduced ? "reduce" : "no-preference",
    });
    const page = await context.newPage();
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
    await page.goto(origin, { waitUntil: "networkidle" });
    await page.locator(".review-link").waitFor();
    await page.evaluate(() => {
      document.querySelector(".review-link").click();
    });
    await page.locator("[data-review-sheet]").waitFor();
    await page.evaluate(
      () =>
        new Promise((resolve) => {
          requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
        }),
    );
    const started = await page.evaluate(() => {
      const anims = document.getAnimations();
      for (const anim of anims) {
        anim.pause();
        anim.currentTime = 0;
      }
      const sheet = document.querySelector("[data-review-sheet]");
      const transform = sheet ? getComputedStyle(sheet).transform : "missing";
      return { count: anims.length, transform };
    });
    if (started.count === 0) {
      throw new Error("no animations to seek");
    }
    const samples = [started.transform];
    for (let frame = 0; frame < FRAMES; frame += 1) {
      const ms = (frame * 1000) / FPS;
      const transform = await page.evaluate((time) => {
        for (const anim of document.getAnimations()) {
          const timing = anim.effect?.getComputedTiming();
          const duration = typeof timing?.duration === "number" ? timing.duration : time;
          anim.pause();
          anim.currentTime = Math.min(time, duration);
        }
        const sheet = document.querySelector("[data-review-sheet]");
        return sheet ? getComputedStyle(sheet).transform : "missing";
      }, ms);
      if (frame === 6 || frame === 18 || frame === 40) {
        samples.push(transform);
      }
      const name = String(frame + 1).padStart(4, "0");
      await page.screenshot({ path: `${dir}/a${name}.png` });
    }
    await context.close();
    return samples;
  }

  const normalSamples = await capture(appDir, false);
  const reducedSamples = await capture(reducedDir, true);
  console.log("normal transforms", normalSamples.join(" | "));
  console.log("reduced transforms", reducedSamples.join(" | "));
  if (reducedSamples.some((value) => value !== "none")) {
    throw new Error(`reduced motion used a transform: ${reducedSamples.join(" | ")}`);
  }
  if (!normalSamples.some((value) => value !== "none" && value !== "missing")) {
    throw new Error(`sheet did not travel: ${normalSamples.join(" | ")}`);
  }

  const stamp = `fontfile=${font}:fontsize=28:fontcolor=0x2B2723:box=1:boxcolor=0xF4F0E8@0.88:boxborderw=10:x=20:y=18`;
  await run("ffmpeg", [
    "-y",
    "-framerate",
    String(FPS),
    "-i",
    `${mockDir}/m%04d.png`,
    "-framerate",
    String(FPS),
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
  await rm(mockDir, { recursive: true, force: true });
  await rm(appDir, { recursive: true, force: true });
  await rm(reducedDir, { recursive: true, force: true });
  console.log("queue-motion: pass");
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
