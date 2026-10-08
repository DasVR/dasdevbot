/**
 * Frame-matched side-by-side: the 4a shell mock against the app, plus the
 * 03-modes gap clip. SD ruling (Oct 8): CD grades this video, not stills.
 *
 * For each viewport (1440x900, 1280x800) and motion mode (full, reduced):
 * - both pages run at DPR 2 and step their own virtual clocks by DT per
 *   frame; nothing is a screencast;
 * - each panel is stamped with its own clock reading (the app's
 *   __shellCap.now(), the mock's summed Clock.step), not frame * DT;
 * - the first action on each side is detected from pixels (first frame
 *   whose bytes differ from frame 0) and the two timelines are aligned on it;
 *   the offset is reported, so a late start is visible, not hidden;
 * - pixel duplicates (a side that repeats a frame while its own animations
 *   say it is moving) are counted per side;
 * - per-frame SSIM between the aligned panels, with the worst frames listed;
 * - G07/G12/G22/G25 are re-measured on both sides at each settled form.
 * The modes clip puts 3a-modes on the left and a GAP card on the right
 * (P1 gap "03-modes not built, due by Oct 14").
 *
 * usage: node scripts/shell-video-match.mjs [--look /workspace/dasdevbot-look]
 *          [--only 1440-full,1280-reduced,modes] [--out docs/review/parity/video]
 * Needs a built apps/desktop/dist (npm run build) and ffmpeg.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, writeFile, readFile, cp, rm, readdir } from "node:fs/promises";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import net from "node:net";
import path from "node:path";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

function arg(name, fallback) {
  const at = process.argv.indexOf(name);
  return at >= 0 && process.argv[at + 1] ? process.argv[at + 1] : fallback;
}

const root = fileURLToPath(new URL("..", import.meta.url));
const desktop = path.join(root, "apps/desktop");
const look = path.resolve(arg("--look", process.env.DASDEVBOT_LOOK || "/workspace/dasdevbot-look"));
const outDir = path.resolve(root, arg("--out", "docs/review/parity/video"));
const only = arg("--only", "")
  .split(",")
  .map((item) => item.trim())
  .filter(Boolean);
const work = "/tmp/shell-video-match";
const DPR = 2;
/** One virtual-clock step per output frame (60fps). */
const DT = 16.667;
const MAX_FRAMES = 720;
const FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

const CLIPS = [
  { id: "1440-full", width: 1440, height: 900, reduced: false },
  { id: "1440-reduced", width: 1440, height: 900, reduced: true },
  { id: "1280-full", width: 1280, height: 800, reduced: false },
  { id: "1280-reduced", width: 1280, height: 800, reduced: true },
].filter((clip) => only.length === 0 || only.includes(clip.id));
const withModes = only.length === 0 || only.includes("modes");

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const chosen = typeof address === "object" && address ? address.port : 0;
      server.close((err) => (err ? reject(err) : resolve(chosen)));
    });
  });
}

function serve(dir) {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      const url = new URL(req.url ?? "/", "http://127.0.0.1");
      const rel = decodeURIComponent(url.pathname).replace(/^\/+/, "");
      const file = path.join(dir, rel);
      if (!file.startsWith(dir)) {
        res.writeHead(403);
        res.end();
        return;
      }
      try {
        const body = await readFile(file);
        const ext = path.extname(file);
        const type =
          ext === ".html" ? "text/html" : ext === ".css" ? "text/css" : ext === ".woff2" ? "font/woff2" : "application/octet-stream";
        res.writeHead(200, { "content-type": type });
        res.end(body);
      } catch {
        res.writeHead(404);
        res.end("missing");
      }
    });
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      resolve({ server, port: typeof address === "object" && address ? address.port : 0 });
    });
  });
}

/** The mocks load fonts and tokens by relative path; mirror that layout. */
async function prepareMocks(dir) {
  await rm(dir, { recursive: true, force: true });
  await mkdir(path.join(dir, "mocks"), { recursive: true });
  await mkdir(path.join(dir, "fonts"), { recursive: true });
  const fonts = path.join(desktop, "src/fonts");
  const pairs = [
    ["figtree-latin-wght-normal.woff2", "Figtree-latin-wght.woff2"],
    ["figtree-latin-ext-wght-normal.woff2", "Figtree-latin-ext-wght.woff2"],
    ["jetbrains-mono-latin-wght-normal.woff2", "JetBrainsMono-latin-wght.woff2"],
    ["jetbrains-mono-latin-ext-wght-normal.woff2", "JetBrainsMono-latin-ext-wght.woff2"],
  ];
  for (const [from, to] of pairs) {
    await cp(path.join(fonts, from), path.join(dir, "fonts", to));
  }
  await cp(path.join(desktop, "src/lib/styles/tokens.css"), path.join(dir, "tokens.css"));
  // The 4a background window embeds thread.html; keep it a plain paper page so
  // both sides frost the same flat desk.
  await writeFile(path.join(dir, "thread.html"), '<!doctype html><html><body style="background:#F6F2EB"></body></html>');
  for (const name of ["4a-shell-morph.html", "3a-modes.html"]) {
    let html = await readFile(path.join(look, "mocks", name), "utf8");
    html = html.replaceAll('format("woff2")', 'format("woff2-variations")');
    await writeFile(path.join(dir, "mocks", name), html);
  }
}

async function waitForHttp(url, child) {
  const deadline = Date.now() + 300_000;
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
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`preview did not respond at ${url}`);
}

function run(cmd, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (chunk) => (out += chunk));
    child.stderr.on("data", (chunk) => (err += chunk));
    child.on("exit", (code) => (code === 0 ? resolve({ out, err }) : reject(new Error(`${cmd} ${code}: ${err.slice(-1500)}`))));
  });
}

function paintFlush(page) {
  return page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined)))));
}

/** True while any animation on the page still has time left. */
function movingProbe() {
  for (const anim of document.getAnimations()) {
    const timing = anim.effect?.getComputedTiming();
    const end = timing ? timing.endTime : 0;
    const current = typeof anim.currentTime === "number" ? anim.currentTime : 0;
    if (typeof end === "number" && Number.isFinite(end) && current + 0.5 < end) {
      return true;
    }
  }
  return false;
}

/** Gap probes, run in either page. Selectors cover the mock and the app. */
function gapProbe() {
  const css = (el) => (el ? getComputedStyle(el) : null);
  const rect = (el) => {
    if (!el) {
      return null;
    }
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.x * 100) / 100, y: Math.round(r.y * 100) / 100, w: Math.round(r.width * 100) / 100, h: Math.round(r.height * 100) / 100 };
  };
  const win = document.querySelector(".win");
  const composer = document.querySelector("#composer, .composer");
  const roster = document.querySelector(".roster");
  // The visible material is the composer's own background or its glass mat.
  const mats = composer ? [composer, ...composer.querySelectorAll(".mat, .glass, .mat-glass")] : [];
  const glass = mats
    .map((el) => ({ el, s: css(el) }))
    .find(({ s }) => s && s.backdropFilter && s.backdropFilter !== "none" && Number(s.opacity) > 0);
  // Both pages hold the composer's three contents as .cl.full / .cl.comp /
  // .cl.pill; the settled form is the one fully shown.
  const layers = composer ? [...composer.querySelectorAll(".cl")] : [];
  const shown = layers.find((el) => Number(getComputedStyle(el).opacity) > 0.99 && el.getClientRects().length > 0);
  const kind = shown ? ["full", "comp", "pill"].find((name) => shown.classList.contains(name)) : "";
  const composerRow = shown ?? composer;
  return {
    form: kind === "comp" ? "companion" : kind || "",
    G07: composer
      ? {
          background: glass ? glass.s.backgroundColor : css(composer).backgroundColor,
          backdrop: glass ? glass.s.backdropFilter : css(composer).backdropFilter,
        }
      : null,
    G12: win ? { rect: rect(win), radius: css(win).borderTopLeftRadius } : null,
    G22: roster ? { background: css(roster).backgroundColor, paddingBottom: css(roster).paddingBottom } : null,
    G25: composerRow
      ? { paddingLeft: css(composerRow).paddingLeft, paddingRight: css(composerRow).paddingRight }
      : null,
  };
}

/** Normalise colours so rgb(237 231 221 / .55) and rgba(237, 231, 221, 0.55) compare equal. */
function normColor(value) {
  if (!value) {
    return value;
  }
  const nums = (value.match(/-?\d*\.?\d+/g) ?? []).map(Number);
  if (/^rgba?\(/.test(value) && nums.length >= 3) {
    const a = nums.length >= 4 ? nums[3] : 1;
    return `rgba(${nums[0]}, ${nums[1]}, ${nums[2]}, ${Math.round(a * 100) / 100})`;
  }
  return value;
}

function compareGaps(mock, app) {
  const verdicts = {};
  const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  if (mock.G07 && app.G07) {
    verdicts.G07 = eq(normColor(mock.G07.background), normColor(app.G07.background)) && mock.G07.backdrop === app.G07.backdrop;
  }
  if (mock.G12 && app.G12) {
    const m = mock.G12.rect;
    const a = app.G12.rect;
    verdicts.G12 = Math.abs(m.x - a.x) < 1 && Math.abs(m.y - a.y) < 1 && Math.abs(m.w - a.w) < 1 && Math.abs(m.h - a.h) < 1 && mock.G12.radius === app.G12.radius;
  }
  if (mock.G22 && app.G22) {
    verdicts.G22 = normColor(mock.G22.background) === normColor(app.G22.background) && mock.G22.paddingBottom === app.G22.paddingBottom;
  }
  if (mock.G25 && app.G25) {
    verdicts.G25 = mock.G25.paddingLeft === app.G25.paddingLeft && mock.G25.paddingRight === app.G25.paddingRight;
  }
  return verdicts;
}

async function hashes(dir, prefix) {
  const names = (await readdir(dir)).filter((name) => name.startsWith(prefix)).sort();
  const out = [];
  for (const name of names) {
    out.push(createHash("sha256").update(await readFile(path.join(dir, name))).digest("hex"));
  }
  return out;
}

function firstAction(list) {
  for (let i = 1; i < list.length; i += 1) {
    if (list[i] !== list[0]) {
      return i;
    }
  }
  return -1;
}

/** A side repeated its previous frame while its animations said it was moving. */
function pixelDuplicates(list, moving) {
  let motion = 0;
  let rest = 0;
  const hits = [];
  for (let i = 1; i < list.length; i += 1) {
    if (list[i] !== list[i - 1]) {
      continue;
    }
    if (moving[i - 1]) {
      motion += 1;
      if (hits.length < 12) {
        hits.push(i);
      }
    } else {
      rest += 1;
    }
  }
  return { motion, rest, hits };
}

/** Capture one side, frame-stepped. Returns per-frame clock and motion data. */
async function captureSide(page, dir, prefix, { start, step, now, done }) {
  const clock = [];
  const moving = [];
  const gaps = [];
  let lastForm = "";
  const cdp = await page.context().newCDPSession(page);
  await page.evaluate(start);
  for (let frame = 0; frame < MAX_FRAMES; frame += 1) {
    if (frame > 0) {
      await page.evaluate(step, DT);
    }
    await paintFlush(page);
    // CDP capture: ~0.4s a frame at DPR 2 on this box, against ~13s through
    // page.screenshot (which re-waits for fonts and re-lays out per call).
    const shot = await cdp.send("Page.captureScreenshot", { format: "jpeg", quality: 90, optimizeForSpeed: true });
    await writeFile(path.join(dir, `${prefix}-${String(frame).padStart(4, "0")}.jpg`), Buffer.from(shot.data, "base64"));
    clock.push(await page.evaluate(now));
    const isMoving = await page.evaluate(movingProbe);
    moving.push(isMoving);
    if (!isMoving) {
      const probe = await page.evaluate(gapProbe);
      if (probe.form && probe.form !== lastForm) {
        gaps.push({ frame, ...probe });
        lastForm = probe.form;
      }
    }
    if (frame > 60 && (await page.evaluate(done))) {
      break;
    }
  }
  return { clock, moving, gaps };
}

/** sendcmd script that rewrites each panel's stamp from its own clock. */
function stampCommands(target, frames, label) {
  return frames
    .map((ms, i) => {
      const t = (i / 60).toFixed(5);
      const text = `${label}  f${i}  ${ms == null ? "-" : `${Math.round(ms)}ms`}`.replace(/:/g, "\\:");
      return `${t} ${target} reinit text='${text}';`;
    })
    .join("\n");
}

async function encodePair(dir, mockFrames, appFrames, offsets, labels, video) {
  // Align on the detected first action by trimming the earlier side's lead-in.
  const lead = Math.max(offsets.mock, offsets.app);
  const trimMock = lead - offsets.mock;
  const trimApp = lead - offsets.app;
  const count = Math.min(mockFrames.length - trimMock, appFrames.length - trimApp);
  const cmdFile = path.join(dir, "stamps.cmd");
  await writeFile(
    cmdFile,
    `${stampCommands("drawtext@l", mockFrames.slice(trimMock, trimMock + count), labels[0])}\n${stampCommands("drawtext@r", appFrames.slice(trimApp, trimApp + count), labels[1])}\n`,
  );
  const draw = (name) =>
    `drawtext@${name}=fontfile=${FONT}:text='':x=24:y=24:fontsize=40:fontcolor=0x2B2723:box=1:boxcolor=0xFBF9F5@0.92:boxborderw=12`;
  await run("ffmpeg", [
    "-y",
    "-framerate", "60", "-start_number", String(trimMock), "-i", path.join(dir, "mock-%04d.jpg"),
    "-framerate", "60", "-start_number", String(trimApp), "-i", path.join(dir, "app-%04d.jpg"),
    "-filter_complex",
    `[0:v]sendcmd=f=${cmdFile},${draw("l")}[left];[1:v]sendcmd=f=${cmdFile},${draw("r")}[right];[left][right]hstack=inputs=2,scale=iw/2:-2`,
    "-frames:v", String(count),
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
    video,
  ]);
  // Per-frame SSIM between the aligned, unstamped panels.
  const ssimFile = path.join(dir, "ssim.log");
  await run("ffmpeg", [
    "-framerate", "60", "-start_number", String(trimMock), "-i", path.join(dir, "mock-%04d.jpg"),
    "-framerate", "60", "-start_number", String(trimApp), "-i", path.join(dir, "app-%04d.jpg"),
    "-lavfi", `[0:v][1:v]ssim=stats_file=${ssimFile}`,
    "-frames:v", String(count),
    "-f", "null", "-",
  ]);
  const ssim = (await readFile(ssimFile, "utf8"))
    .trim()
    .split("\n")
    .map((line) => Number((line.match(/All:([0-9.]+)/) ?? [])[1]));
  return { count, trimMock, trimApp, ssim };
}

function summarise(ssim) {
  const valid = ssim.filter((value) => Number.isFinite(value));
  const mean = valid.reduce((sum, value) => sum + value, 0) / Math.max(1, valid.length);
  const worst = ssim
    .map((value, frame) => ({ frame, value }))
    .filter(({ value }) => Number.isFinite(value))
    .sort((a, b) => a.value - b.value)
    .slice(0, 8)
    .map(({ frame, value }) => ({ frame, ssim: Math.round(value * 10000) / 10000 }));
  return { mean: Math.round(mean * 10000) / 10000, min: worst[0]?.ssim ?? null, worst };
}

async function captureClip(browser, origin, mockOrigin, clip) {
  const dir = path.join(work, clip.id);
  await rm(dir, { recursive: true, force: true });
  await mkdir(dir, { recursive: true });
  const contextOpts = {
    viewport: { width: clip.width, height: clip.height },
    deviceScaleFactor: DPR,
    timezoneId: "America/New_York",
    reducedMotion: clip.reduced ? "reduce" : "no-preference",
  };
  const mockCtx = await browser.newContext(contextOpts);
  const appCtx = await browser.newContext(contextOpts);
  try {
    const mockPage = await mockCtx.newPage();
    const appPage = await appCtx.newPage();
    await mockPage.goto(`${mockOrigin}/mocks/4a-shell-morph.html?${clip.reduced ? "capture&rm" : "capture"}`, { waitUntil: "load", timeout: 180_000 });
    await appPage.goto(`${origin}/?shellCapture=1`, { waitUntil: "load", timeout: 180_000 });
    await appPage.locator(".wordmark").waitFor();
    await mockPage.waitForFunction(() => typeof window.__cap?.start === "function");
    await appPage.waitForFunction(() => typeof window.__shellCap?.start === "function");
    await mockPage.evaluate(() => document.fonts.ready);
    await appPage.evaluate(() => document.fonts.ready);
    // The mock exposes only step(); its clock reading is the sum of steps.
    await mockPage.evaluate(() => {
      window.__matchClock = 0;
    });
    const mock = await captureSide(mockPage, dir, "mock", {
      start: () => window.__cap.start(),
      step: (dt) => {
        window.__matchClock += dt;
        return window.__cap.step(dt);
      },
      now: () => window.__matchClock,
      done: () => window.__cap.done,
    });
    const app = await captureSide(appPage, dir, "app", {
      start: () => window.__shellCap.start(),
      step: (dt) => window.__shellCap.step(dt),
      now: () => window.__shellCap.now(),
      done: () => !window.__shellCap.running() && window.__shellCap.pending() === 0,
    });
    const mockHashes = await hashes(dir, "mock-");
    const appHashes = await hashes(dir, "app-");
    const offsets = { mock: firstAction(mockHashes), app: firstAction(appHashes) };
    if (offsets.mock < 0 || offsets.app < 0) {
      throw new Error(`${clip.id}: no first action detected ${JSON.stringify(offsets)}`);
    }
    // The app stamp must be its own clock and must advance one DT per frame.
    const drift = app.clock.reduce((worst, ms, i) => Math.max(worst, Math.abs(ms - app.clock[0] - i * DT)), 0);
    await mkdir(outDir, { recursive: true });
    const video = path.join(outDir, `shell-${clip.id}.mp4`);
    const encoded = await encodePair(dir, mock.clock, app.clock, offsets, ["Mock 4a", "App"], video);
    const gapRows = [];
    for (const mockGap of mock.gaps) {
      const appGap = app.gaps.find((item) => item.form === mockGap.form);
      if (appGap) {
        gapRows.push({ form: mockGap.form, mockFrame: mockGap.frame, appFrame: appGap.frame, mock: mockGap, app: appGap, pass: compareGaps(mockGap, appGap) });
      }
    }
    return {
      clip: clip.id,
      video: path.relative(root, video),
      frames: { mock: mockHashes.length, app: appHashes.length, encoded: encoded.count },
      firstAction: { ...offsets, alignedBy: { trimMock: encoded.trimMock, trimApp: encoded.trimApp } },
      appClockDriftMs: Math.round(drift * 1000) / 1000,
      duplicates: { mock: pixelDuplicates(mockHashes, mock.moving), app: pixelDuplicates(appHashes, app.moving) },
      ssim: summarise(encoded.ssim),
      gaps: gapRows,
    };
  } finally {
    await mockCtx.close();
    await appCtx.close();
  }
}

/** 03-modes: mock on the left, the declared gap on the right. */
async function captureModes(browser, mockOrigin) {
  const dir = path.join(work, "modes");
  await rm(dir, { recursive: true, force: true });
  await mkdir(dir, { recursive: true });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: DPR, timezoneId: "America/New_York" });
  try {
    const page = await ctx.newPage();
    await page.goto(`${mockOrigin}/mocks/3a-modes.html?capture`, { waitUntil: "load", timeout: 180_000 });
    await page.waitForFunction(() => typeof window.__cap?.start === "function");
    await page.evaluate(() => {
      window.__matchClock = 0;
    });
    const side = await captureSide(page, dir, "mock", {
      start: () => window.__cap.start(),
      step: (dt) => {
        window.__matchClock += dt;
        return window.__cap.step(dt);
      },
      now: () => window.__matchClock,
      done: () => window.__cap.done,
    });
    const cmdFile = path.join(dir, "stamps.cmd");
    await writeFile(cmdFile, `${stampCommands("drawtext@l", side.clock, "Mock 3a modes")}\n`);
    const video = path.join(outDir, "modes-gap.mp4");
    const gapText = "GAP P1  03-modes not built (due by Oct 14)";
    await run("ffmpeg", [
      "-y",
      "-framerate", "60", "-i", path.join(dir, "mock-%04d.jpg"),
      "-f", "lavfi", "-i", `color=c=0xF6F2EB:s=${1440 * DPR}x${900 * DPR}:r=60`,
      "-filter_complex",
      `[0:v]sendcmd=f=${cmdFile},drawtext@l=fontfile=${FONT}:text='':x=24:y=24:fontsize=40:fontcolor=0x2B2723:box=1:boxcolor=0xFBF9F5@0.92:boxborderw=12[left];` +
        `[1:v]drawtext=fontfile=${FONT}:text='${gapText}':x=(w-tw)/2:y=(h-th)/2:fontsize=64:fontcolor=0x2B2723[right];[left][right]hstack=inputs=2:shortest=1,scale=iw/2:-2`,
      "-frames:v", String(side.clock.length),
      "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
      video,
    ]);
    return { clip: "modes", video: path.relative(root, video), frames: side.clock.length, gap: "03-modes not built, due by Oct 14" };
  } finally {
    await ctx.close();
  }
}

const viteBin = path.join(desktop, "node_modules/vite/bin/vite.js");
const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
const preview = spawn("node", [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: desktop,
  stdio: ["ignore", "ignore", "pipe"],
});
preview.stderr.on("data", () => {});
const mockDir = path.join(work, "look");
await prepareMocks(mockDir);
const mockServer = await serve(mockDir);
const mockOrigin = `http://127.0.0.1:${mockServer.port}`;
await waitForHttp(origin, preview);
const browser = await chromium.launch();
// Clips run one --only at a time on a busy box; merge into the existing report.
await mkdir(outDir, { recursive: true });
const reportPath = path.join(outDir, "match-report.json");
let report = { dpr: DPR, dtMs: DT, clips: [] };
try {
  report = JSON.parse(await readFile(reportPath, "utf8"));
} catch {
  // first clip
}
const keep = (result) => {
  report.clips = [...report.clips.filter((item) => item.clip !== result.clip), result];
  report.generatedAt = new Date().toISOString();
};
try {
  for (const clip of CLIPS) {
    const started = Date.now();
    const result = await captureClip(browser, origin, mockOrigin, clip);
    result.wallSeconds = Math.round((Date.now() - started) / 1000);
    result.commit = process.env.MATCH_COMMIT || "";
    keep(result);
    console.log(JSON.stringify({ clip: result.clip, frames: result.frames, firstAction: result.firstAction, ssim: result.ssim.mean, gaps: result.gaps.map((row) => ({ form: row.form, ...row.pass })) }));
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  }
  if (withModes) {
    keep(await captureModes(browser, mockOrigin));
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  }
  console.log(`wrote ${path.relative(root, outDir)}/match-report.json`);
} finally {
  await browser.close();
  preview.kill("SIGTERM");
  mockServer.server.close();
}
