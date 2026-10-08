/**
 * Frame-matched side-by-side: the design video (shell-full-companion-pill.mp4)
 * against the app, plus the 03-modes gap clip. SD ruling (Oct 8): CD grades
 * this video, not stills. The video has no reduced-motion cut, so reduced
 * clips use the 4a mock page (?capture&rm), which the video was cut from.
 * The source videos are never committed; only frames of them are.
 *
 * For each viewport (1440x900, 1280x800) and motion mode (full, reduced):
 * - the app runs at DPR 2 and steps its virtual clock by DT per frame;
 *   nothing is a screencast; frames are compared at the video's CSS size;
 * - each panel is stamped with its own clock (video pts, the app's
 *   __shellCap.now(), or the mock's summed Clock.step), not frame * DT;
 * - the first action on each side is detected from pixels (first frame with
 *   more than CHANGED_PX pixels changed against the frame before, from frame
 *   2 on, since an encoded frame 0 is a keyframe; frame-diff.py) and
 *   the two timelines are aligned on it;
 *   the offset is reported, so a late start is visible, not hidden;
 * - pixel duplicates (a repeated frame while that side is moving) are
 *   counted per side, including encoder repeats in the video;
 * - per-frame SSIM between the aligned panels, with the worst frames listed;
 * - G07/G12/G22/G25 are re-measured against the 4a mock DOM at each settled form.
 * The modes clip puts modes-focus-away-review.mp4 on the left and a GAP card on the right
 * (P1 gap "03-modes not built, due by Oct 14").
 *
 * usage: node scripts/shell-video-match.mjs [--look /workspace/dasdevbot-look]
 *          [--only 1440-full,1280-reduced,modes] [--out docs/review/parity/video]
 * Needs a built apps/desktop/dist (npm run build) and ffmpeg.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdir, writeFile, readFile, cp, rm, readdir, rename } from "node:fs/promises";
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
/** The design video CD grades against (never committed; only frames of it are published). */
const referenceVideo = path.resolve(arg("--video", "/workspace/vid/shell-full-companion-pill.mp4"));
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
  // The 4a background window embeds the real thread.html?capture under its
  // frost (mock 4a:1238). The app's stage draws the same thread (UID 4), so
  // the reference keeps it rather than a flat page.
  const thread = await readFile(path.join(look, "thread.html"), "utf8");
  await writeFile(path.join(dir, "thread.html"), thread.replaceAll('format("woff2")', 'format("woff2-variations")'));
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

/**
 * Step one page frame by frame. `scale` is the capture scale: DPR for the
 * app (CDP clip.scale, so the JPEG is real 2x pixels; without a clip CDP
 * returns CSS-size images even at DPR 2), 0 to only probe (no screenshots).
 */
async function captureSide(page, dir, prefix, { start, step, now, done }, { scale = DPR, width, height } = {}) {
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
    if (scale > 0) {
      const shot = await cdp.send("Page.captureScreenshot", {
        format: "jpeg",
        quality: 92,
        optimizeForSpeed: true,
        clip: { x: 0, y: 0, width, height, scale },
      });
      await writeFile(path.join(dir, `${prefix}-${String(frame).padStart(4, "0")}.jpg`), Buffer.from(shot.data, "base64"));
    }
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

/** Reference frames from the design video (1440x900, 60fps), top-left crop for smaller viewports. */
async function extractVideo(dir, width, height) {
  await run("ffmpeg", ["-y", "-i", referenceVideo, "-vf", `crop=${width}:${height}:0:0`, "-vsync", "0", "-q:v", "2", path.join(dir, "ref-%04d.jpg")]);
  const names = (await readdir(dir)).filter((name) => name.startsWith("ref-")).sort();
  // ffmpeg numbers from 1; renumber from 0 to match the app side.
  for (let i = 0; i < names.length; i += 1) {
    await rename(path.join(dir, names[i]), path.join(dir, `ref-${String(i).padStart(4, "0")}.jpg`));
  }
  return names.length;
}

/** Pixel change per frame (numpy helper), at the reference's CSS size. */
async function frameChanges(dir, prefix, width, height) {
  const { out } = await run("python3", [path.join(root, "scripts/frame-diff.py"), dir, prefix, String(width), String(height)]);
  return JSON.parse(out);
}

/** Noise floor: a frame "changed" if more than this many CSS pixels moved. */
const CHANGED_PX = 24;

/**
 * First frame that moves against the frame before it. Frame 1 is skipped: an
 * encoded video's frame 0 is a keyframe and 0->1 differs by encoder noise
 * alone (~335 px on the design video), which read as a first action at f1.
 */
function firstChange(prev) {
  return prev.findIndex((count, i) => i >= 2 && count > CHANGED_PX);
}

/** Repeats of the previous frame while that side was moving. */
function repeats(prev, moving) {
  let motion = 0;
  let rest = 0;
  const hits = [];
  for (let i = 1; i < prev.length; i += 1) {
    if (prev[i] > CHANGED_PX) {
      continue;
    }
    if (moving(i)) {
      motion += 1;
      if (hits.length < 16) {
        hits.push(i);
      }
    } else {
      rest += 1;
    }
  }
  return { motion, rest, hits };
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

async function encodePair(dir, refPrefix, refStamps, appStamps, offsets, labels, size, video) {
  // Align on the detected first action by trimming the later side's lead-in
  // (F3: trimming the earlier side put ref f18 at clip f17 and app f19 at
  // clip f19, shifting the full-clip morphs by +2).
  const lead = Math.min(offsets.ref, offsets.app);
  const trimRef = offsets.ref - lead;
  const trimApp = offsets.app - lead;
  const count = Math.min(refStamps.length - trimRef, appStamps.length - trimApp);
  const cmdFile = path.join(dir, "stamps.cmd");
  await writeFile(
    cmdFile,
    `${stampCommands("drawtext@l", refStamps.slice(trimRef, trimRef + count), labels[0])}\n${stampCommands("drawtext@r", appStamps.slice(trimApp, trimApp + count), labels[1])}\n`,
  );
  const draw = (name) =>
    `drawtext@${name}=fontfile=${FONT}:text='':x=16:y=16:fontsize=22:fontcolor=0x2B2723:box=1:boxcolor=0xFBF9F5@0.92:boxborderw=8`;
  // The app is captured at DPR 2; both panels are shown and compared at the
  // reference's CSS size (the design video is 1x).
  const inputs = [
    "-framerate", "60", "-start_number", String(trimRef), "-i", path.join(dir, `${refPrefix}-%04d.jpg`),
    "-framerate", "60", "-start_number", String(trimApp), "-i", path.join(dir, "app-%04d.jpg"),
  ];
  const fit = `scale=${size.width}:${size.height}:flags=area`;
  await run("ffmpeg", [
    "-y", ...inputs,
    "-filter_complex",
    `[0:v]${fit},sendcmd=f=${cmdFile},${draw("l")}[left];[1:v]${fit},sendcmd=f=${cmdFile},${draw("r")}[right];[left][right]hstack=inputs=2`,
    "-frames:v", String(count),
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "16",
    video,
  ]);
  const ssimFile = path.join(dir, "ssim.log");
  await run("ffmpeg", [
    ...inputs,
    "-lavfi", `[0:v]${fit}[a];[1:v]${fit}[b];[a][b]ssim=stats_file=${ssimFile}`,
    "-frames:v", String(count),
    "-f", "null", "-",
  ]);
  const ssim = (await readFile(ssimFile, "utf8"))
    .trim()
    .split("\n")
    .map((line) => Number((line.match(/All:([0-9.]+)/) ?? [])[1]));
  return { count, trimRef, trimApp, ssim };
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
  const size = { width: clip.width, height: clip.height };
  const contextOpts = {
    viewport: size,
    deviceScaleFactor: DPR,
    timezoneId: "America/New_York",
    reducedMotion: clip.reduced ? "reduce" : "no-preference",
  };
  // The mock page always runs: it is the reference for reduced motion (the
  // design video has no reduced cut) and the DOM for the gap probes.
  const mockCtx = await browser.newContext({ ...contextOpts, deviceScaleFactor: 1 });
  const appCtx = await browser.newContext(contextOpts);
  try {
    const mockPage = await mockCtx.newPage();
    const appPage = await appCtx.newPage();
    await mockPage.goto(`${mockOrigin}/mocks/4a-shell-morph.html?${clip.reduced ? "capture&rm" : "capture"}`, { waitUntil: "load", timeout: 180_000 });
    await appPage.goto(`${origin}/?shellCapture=1`, { waitUntil: "load", timeout: 180_000 });
    await appPage.locator(".wordmark").waitFor({ timeout: 180_000 });
    await mockPage.waitForFunction(() => typeof window.__cap?.start === "function");
    await appPage.waitForFunction(() => typeof window.__shellCap?.start === "function");
    await mockPage.evaluate(() => document.fonts.ready);
    await appPage.evaluate(() => document.fonts.ready);
    await mockPage.evaluate(() => {
      window.__matchClock = 0;
    });
    const mock = await captureSide(
      mockPage,
      dir,
      "mock",
      {
        start: () => window.__cap.start(),
        step: (dt) => {
          window.__matchClock += dt;
          return window.__cap.step(dt);
        },
        now: () => window.__matchClock,
        done: () => window.__cap.done,
      },
      { scale: clip.reduced ? 1 : 0, ...size },
    );
    const app = await captureSide(
      appPage,
      dir,
      "app",
      {
        start: () => window.__shellCap.start(),
        step: (dt) => window.__shellCap.step(dt),
        now: () => window.__shellCap.now(),
        done: () => !window.__shellCap.running() && window.__shellCap.pending() === 0,
      },
      { scale: DPR, ...size },
    );
    let refPrefix = "mock";
    let refStamps = mock.clock;
    let refLabel = "Mock 4a (reduced; the video has no reduced cut)";
    if (!clip.reduced) {
      const n = await extractVideo(dir, clip.width, clip.height);
      refPrefix = "ref";
      refStamps = Array.from({ length: n }, (_, i) => (i * 1000) / 60);
      refLabel = `Video ${path.basename(referenceVideo)}${clip.width < 1440 ? " (top-left crop)" : ""}`;
    }
    const refChange = await frameChanges(dir, refPrefix, clip.width, clip.height);
    const appChange = await frameChanges(dir, "app", clip.width, clip.height);
    const offsets = { ref: firstChange(refChange.prev), app: firstChange(appChange.prev) };
    if (offsets.ref < 0 || offsets.app < 0) {
      throw new Error(`${clip.id}: no first action detected ${JSON.stringify(offsets)}`);
    }
    const drift = app.clock.reduce((worst, ms, i) => Math.max(worst, Math.abs(ms - app.clock[0] - i * DT)), 0);
    await mkdir(outDir, { recursive: true });
    const video = path.join(outDir, `shell-${clip.id}.mp4`);
    const encoded = await encodePair(dir, refPrefix, refStamps, app.clock, offsets, [refLabel, "App (DPR 2)"], size, video);
    // A reference frame is "moving" if either neighbour changed; the app knows from its animations.
    const refMoving = (i) => (refChange.prev[i - 1] ?? 0) > CHANGED_PX || (refChange.prev[i + 1] ?? 0) > CHANGED_PX;
    const gapRows = [];
    for (const mockGap of mock.gaps) {
      const appGap = app.gaps.find((item) => item.form === mockGap.form);
      if (appGap) {
        gapRows.push({ form: mockGap.form, mockFrame: mockGap.frame, appFrame: appGap.frame, mock: mockGap, app: appGap, pass: compareGaps(mockGap, appGap) });
      }
    }
    // Publish a few aligned stills (reference | app) at the worst-SSIM frames.
    const stills = [];
    const worst = summarise(encoded.ssim).worst.slice(0, 3);
    for (const { frame } of worst) {
      const still = path.join(outDir, `shell-${clip.id}-f${frame}.png`);
      await run("ffmpeg", ["-y", "-i", video, "-vf", `select=eq(n\\,${frame})`, "-frames:v", "1", still]);
      stills.push(path.relative(root, still));
    }
    return {
      clip: clip.id,
      reference: clip.reduced ? "mock 4a-shell-morph.html ?capture&rm" : path.basename(referenceVideo),
      video: path.relative(root, video),
      stills,
      frames: { reference: refStamps.length, app: app.clock.length, encoded: encoded.count },
      firstAction: { ...offsets, alignedBy: { trimRef: encoded.trimRef, trimApp: encoded.trimApp } },
      appClockDriftMs: Math.round(drift * 1000) / 1000,
      duplicates: {
        reference: repeats(refChange.prev, refMoving),
        app: repeats(appChange.prev, (i) => app.moving[i - 1]),
      },
      ssim: summarise(encoded.ssim),
      gaps: gapRows,
    };
  } finally {
    await mockCtx.close();
    await appCtx.close();
  }
}

/** 03-modes: the design video on the left, the declared gap on the right. */
async function captureModes() {
  const dir = path.join(work, "modes");
  await rm(dir, { recursive: true, force: true });
  await mkdir(dir, { recursive: true });
  const source = path.resolve(arg("--modes-video", "/workspace/vid/modes-focus-away-review.mp4"));
  const { out } = await run("ffprobe", ["-v", "error", "-select_streams", "v", "-count_frames", "-show_entries", "stream=nb_read_frames,width,height", "-of", "csv=p=0", source]);
  const [width, height, frames] = out.trim().split(",").map(Number);
  const cmdFile = path.join(dir, "stamps.cmd");
  await writeFile(cmdFile, `${stampCommands("drawtext@l", Array.from({ length: frames }, (_, i) => (i * 1000) / 60), `Video ${path.basename(source)}`)}\n`);
  await mkdir(outDir, { recursive: true });
  const video = path.join(outDir, "modes-gap.mp4");
  const gapText = "GAP P1  03-modes not built (due by Oct 14)";
  await run("ffmpeg", [
    "-y",
    "-i", source,
    "-f", "lavfi", "-i", `color=c=0xF6F2EB:s=${width}x${height}:r=60`,
    "-filter_complex",
    `[0:v]sendcmd=f=${cmdFile},drawtext@l=fontfile=${FONT}:text='':x=16:y=16:fontsize=22:fontcolor=0x2B2723:box=1:boxcolor=0xFBF9F5@0.92:boxborderw=8[left];` +
      `[1:v]drawtext=fontfile=${FONT}:text='${gapText}':x=(w-tw)/2:y=(h-th)/2:fontsize=36:fontcolor=0x2B2723[right];[left][right]hstack=inputs=2:shortest=1`,
    "-frames:v", String(frames),
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "16",
    video,
  ]);
  return { clip: "modes", reference: path.basename(source), video: path.relative(root, video), frames, gap: "03-modes not built, due by Oct 14" };
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
    keep(await captureModes());
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  }
  console.log(`wrote ${path.relative(root, outDir)}/match-report.json`);
} finally {
  await browser.close();
  preview.kill("SIGTERM");
  mockServer.server.close();
}
