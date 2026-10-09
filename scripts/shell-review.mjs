/**
 * Review stills and the mock-vs-app morph.
 *
 * Stills: 1280×800 at 2x, America/New_York, pointer off the roster rows.
 * Motion: mock on the left, app on the right, same width, 60fps, the same
 * full → companion → pill → full sequence. Both clocks are frame-stepped
 * from the same input event. Each panel burns its frame and millisecond.
 * A second file, shell-reduced.mp4, repeats the sequence with
 * prefers-reduced-motion. Pass --video to skip the stills.
 *
 * The pill is the browser stage. Linux WebKit will not show a transparent
 * window, so the glass is the CSS recipe over the desk, not an OS blur.
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

const args = process.argv.slice(2).filter((arg) => arg !== "--video");
const videoOnly = process.argv.includes("--video");
const mockHtml = args[0];
if (!mockHtml) {
  console.error("usage: shell-review.mjs <4a-shell-morph.html> [--video]");
  process.exit(1);
}

const root = fileURLToPath(new URL("..", import.meta.url));
const desktop = path.join(root, "apps/desktop");
const outDir = path.join(root, "docs/review/shell");
const frameDir = "/tmp/shell-review-frames";
const mockRoot = "/tmp/shell-review-mock";
const STAGE_W = 1440;
const STAGE_H = 900;
/** One virtual-clock step per output frame. Not 1000/60, and not a 30fps screencast. */
const DT = 16.667;

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

function serve(dir) {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      const url = new URL(req.url ?? "/", "http://127.0.0.1");
      const rel = decodeURIComponent(url.pathname).replace(/^\/+/, "");
      const file = path.join(dir, rel === "" ? "page/index.html" : rel);
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

async function prepareMock() {
  await rm(mockRoot, { recursive: true, force: true });
  await mkdir(path.join(mockRoot, "page"), { recursive: true });
  await mkdir(path.join(mockRoot, "fonts"), { recursive: true });
  const fonts = path.join(desktop, "src/fonts");
  await cp(path.join(fonts, "figtree-latin-wght-normal.woff2"), path.join(mockRoot, "fonts/Figtree-latin-wght.woff2"));
  await cp(path.join(fonts, "figtree-latin-ext-wght-normal.woff2"), path.join(mockRoot, "fonts/Figtree-latin-ext-wght.woff2"));
  await cp(path.join(fonts, "jetbrains-mono-latin-wght-normal.woff2"), path.join(mockRoot, "fonts/JetBrainsMono-latin-wght.woff2"));
  await cp(
    path.join(fonts, "jetbrains-mono-latin-ext-wght-normal.woff2"),
    path.join(mockRoot, "fonts/JetBrainsMono-latin-ext-wght.woff2"),
  );
  await cp(path.join(desktop, "src/lib/styles/tokens.css"), path.join(mockRoot, "tokens.css"));
  await writeFile(
    path.join(mockRoot, "thread.html"),
    "<!doctype html><html><body style=\"background:#F6F2EB\"></body></html>",
  );
  let html = await readFile(mockHtml, "utf8");
  html = html.replaceAll('format("woff2")', 'format("woff2-variations")');
  await writeFile(path.join(mockRoot, "page/index.html"), html);
}

function launch() {
  return chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
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
      // Preview is not accepting connections yet.
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`preview did not respond at ${url}`);
}

const FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

function panelFilter(label) {
  const name = `drawtext=fontfile=${FONT}:text='${label}':x=24:y=24:fontsize=28:fontcolor=0x2B2723:box=1:boxcolor=0xFBF9F5@0.92:boxborderw=10`;
  const stamp = `drawtext=fontfile=${FONT}:text='f%{n} %{eif\\:n*16.667\\:d}ms':x=w-tw-24:y=24:fontsize=22:fontcolor=0x2B2723:box=1:boxcolor=0xFBF9F5@0.92:boxborderw=8`;
  return `${name},${stamp}`;
}

function encode(video, leftLabel, rightLabel) {
  return new Promise((resolve, reject) => {
    const ffmpeg = spawn(
      "ffmpeg",
      [
        "-y",
        "-framerate",
        "60",
        "-i",
        path.join(frameDir, "mock-%04d.jpg"),
        "-framerate",
        "60",
        "-i",
        path.join(frameDir, "app-%04d.jpg"),
        "-filter_complex",
        `[0:v]${panelFilter(leftLabel)}[left];[1:v]${panelFilter(rightLabel)}[right];[left][right]hstack=inputs=2`,
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-fps_mode",
        "passthrough",
        video,
      ],
      { stdio: ["ignore", "ignore", "pipe"] },
    );
    let err = "";
    ffmpeg.stderr.on("data", (chunk) => {
      err += chunk.toString();
    });
    ffmpeg.on("exit", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(err.slice(-2000)));
      }
    });
  });
}

async function frameHashes(prefix) {
  const names = (await readdir(frameDir)).filter((name) => name.startsWith(prefix)).sort();
  const hashes = [];
  for (const name of names) {
    const body = await readFile(path.join(frameDir, name));
    hashes.push(createHash("sha256").update(body).digest("hex"));
  }
  return hashes;
}

function countDuplicates(hashes) {
  let duplicateFrames = 0;
  let isolated = 0;
  let run = 0;
  for (let i = 1; i < hashes.length; i += 1) {
    if (hashes[i] === hashes[i - 1]) {
      duplicateFrames += 1;
      run += 1;
    } else if (run === 1) {
      isolated += 1;
      run = 0;
    } else {
      run = 0;
    }
  }
  if (run === 1) {
    isolated += 1;
  }
  return { frames: hashes.length, duplicateFrames, isolated };
}

async function duplicateReport() {
  const mockHashes = await frameHashes("mock-");
  const appHashes = await frameHashes("app-");
  let composite = 0;
  for (let i = 1; i < mockHashes.length; i += 1) {
    if (mockHashes[i] === mockHashes[i - 1] && appHashes[i] === appHashes[i - 1]) {
      composite += 1;
    }
  }
  return { mock: countDuplicates(mockHashes), app: countDuplicates(appHashes), composite, frames: mockHashes.length };
}

function fieldOpacity(value) {
  const parts = value.split("|");
  const raw = parts.length >= 5 ? parts[4] : parts[parts.length - 1];
  if (raw == null || raw === "") {
    return 1;
  }
  const opacity = Number(raw);
  return Number.isFinite(opacity) ? opacity : 1;
}

function movedPx(before, after) {
  if (fieldOpacity(before) === 0 && fieldOpacity(after) === 0) {
    return false;
  }
  const read = (value) => (value.match(/-?\d+(?:\.\d+)?/g) ?? []).map(Number);
  const left = read(before);
  const right = read(after);
  const count = Math.max(left.length, right.length);
  for (let i = 0; i < count; i += 1) {
    if (Math.abs((left[i] ?? 0) - (right[i] ?? 0)) >= 1) {
      return true;
    }
  }
  return false;
}

function countMotionDuplicates(hashes, samples, side) {
  let motionDuplicates = 0;
  let restDuplicates = 0;
  const hits = [];
  for (let i = 1; i < hashes.length; i += 1) {
    if (hashes[i] !== hashes[i - 1]) {
      continue;
    }
    const prev = samples[i - 1][side];
    const cur = samples[i][side];
    const styleMoved =
      movedPx(prev.cursor, cur.cursor) || movedPx(prev.win, cur.win) || movedPx(prev.composer, cur.composer);
    if (styleMoved) {
      motionDuplicates += 1;
      if (hits.length < 8) {
        hits.push({
          frame: i,
          win: [prev.win, cur.win],
          cursor: [prev.cursor, cur.cursor],
          composer: [prev.composer, cur.composer],
        });
      }
    } else {
      restDuplicates += 1;
    }
  }
  return { motionDuplicates, restDuplicates, hits };
}

function readFrameCount(video) {
  return new Promise((resolve, reject) => {
    const probe = spawn(
      "ffprobe",
      ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=nb_frames,avg_frame_rate", "-of", "json", video],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    let out = "";
    let err = "";
    probe.stdout.on("data", (chunk) => {
      out += chunk.toString();
    });
    probe.stderr.on("data", (chunk) => {
      err += chunk.toString();
    });
    probe.on("exit", (code) => {
      if (code !== 0) {
        reject(new Error(err.slice(-500)));
        return;
      }
      const parsed = JSON.parse(out);
      resolve(parsed.streams[0]);
    });
  });
}

async function shot(page, name) {
  await page.screenshot({
    path: path.join(outDir, name),
    animations: "allow",
  });
  console.log(`wrote ${name}`);
}

async function park(page, where) {
  if (where === "title") {
    const box = await page.locator(".tbar").boundingBox();
    if (!box) {
      throw new Error("titlebar missing");
    }
    await page.mouse.move(box.x + 90, box.y + box.height / 2);
    return;
  }
  await page.mouse.move(48, 16);
}

async function assertChrome(page, form) {
  const report = await page.evaluate((which) => {
    const composer = document.querySelector(".composer");
    const rows = [...document.querySelectorAll(".roster .sub")].map((node) => ({
      text: node.textContent?.trim() ?? "",
      wrap: getComputedStyle(node).whiteSpace,
    }));
    const mic = document.querySelector(".cl.full .mic");
    const pill = document.querySelector(".cl.pill");
    const win = document.querySelector(".win");
    const stream = document.querySelector(".stream-slot");
    const day = document.querySelector(".stage-thread .day");
    return {
      composerBlur: composer ? getComputedStyle(composer).backdropFilter : "",
      paperMat: Boolean(document.querySelector(".mat.paper")),
      tokenNode: Boolean(document.querySelector(".tokens")),
      extras: Boolean(document.querySelector(".extras, .modes, .review-link")),
      day: day?.textContent?.trim() ?? "",
      reply: document.querySelector(".stage-thread")?.textContent?.includes("try/finally") ?? false,
      streamInert: stream instanceof HTMLElement ? stream.inert : false,
      winInert: win instanceof HTMLElement ? win.inert : false,
      micColor: mic ? getComputedStyle(mic).color : "",
      pillColor: pill ? getComputedStyle(pill).color : "",
      risk: getComputedStyle(document.documentElement).getPropertyValue("--risk-external").trim(),
      winTransform: win ? getComputedStyle(win).transform : "",
      rows,
      form: window.__shellStage?.form(),
      which,
    };
  }, form);
  if (report.form !== form) {
    throw new Error(`expected ${form}, painted ${report.form}`);
  }
  if (!report.composerBlur.includes("blur")) {
    throw new Error(`${form} composer is not glass: ${report.composerBlur}`);
  }
  if (report.paperMat) {
    throw new Error(`${form} still paints a paper mat`);
  }
  if (report.tokenNode || report.extras) {
    throw new Error(`${form} still has lease tokens or titlebar extras`);
  }
  if (form === "full" && (report.day !== "Today · 10:38 AM" || !report.reply)) {
    throw new Error(`full thread is not the mock story: ${report.day}`);
  }
  if (form === "companion" && !report.streamInert) {
    throw new Error("companion collapse is not inert");
  }
  if (form === "pill" && !report.winInert) {
    throw new Error("pill window is not inert");
  }
  if (form === "full" && (report.streamInert || report.winInert)) {
    throw new Error("full form left the thread inert");
  }
  if (report.winTransform !== "none") {
    throw new Error(`window transform ${report.winTransform}`);
  }
  for (const row of report.rows) {
    if (row.wrap !== "nowrap") {
      throw new Error(`lease wraps: ${row.text}`);
    }
    if (row.text.includes("tok") || row.text.includes("/")) {
      throw new Error(`lease label still shows a token count: ${row.text}`);
    }
  }
  const risk = report.risk.toLowerCase();
  if (risk && (report.micColor.includes(risk) || report.pillColor.includes(risk))) {
    throw new Error(`mic uses a status color ${report.micColor} / ${report.pillColor}`);
  }
  console.log(`chrome ${form} ok`);
}

const viteBin = path.join(desktop, "node_modules/vite/bin/vite.js");
const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
const preview = spawn("node", [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: desktop,
  stdio: ["ignore", "ignore", "pipe"],
});
preview.stderr.on("data", () => {});

await prepareMock();
const mock = await serve(mockRoot);
await waitForHttp(origin, preview);
await mkdir(outDir, { recursive: true });

function pressMark() {
  const el = document.querySelector(".is-press");
  if (!el) {
    return "";
  }
  const label = el.getAttribute("aria-label") || "";
  if (el.id === "toComp" || label === "Companion window") {
    return "companion";
  }
  if (el.id === "toPill" || label === "Float as a pill") {
    return "pill";
  }
  if (el.id === "composer" || el.classList.contains("composer")) {
    return "composer";
  }
  return el.id || label || "";
}

async function capturePair(browser, { reduced, minFrames }) {
  await rm(frameDir, { recursive: true, force: true });
  await mkdir(frameDir, { recursive: true });
  const wide = await browser.newContext({
    viewport: { width: STAGE_W, height: STAGE_H },
    timezoneId: "America/New_York",
    deviceScaleFactor: 1,
    reducedMotion: reduced ? "reduce" : "no-preference",
  });
  const mockPage = await wide.newPage();
  const appPage = await wide.newPage();
  const mockQuery = reduced ? "capture&rm" : "capture";
  await mockPage.goto(`http://127.0.0.1:${mock.port}/page/index.html?${mockQuery}`, { waitUntil: "networkidle" });
  await appPage.goto(`${origin}/?shellCapture=1`, { waitUntil: "networkidle" });
  await appPage.locator(".wordmark").waitFor();
  await mockPage.waitForFunction(() => typeof window.__cap?.start === "function");
  await appPage.waitForFunction(() => typeof window.__shellCap?.start === "function");
  const motion = await Promise.all([
    mockPage.evaluate(() => document.documentElement.classList.contains("rm") || matchMedia("(prefers-reduced-motion: reduce)").matches),
    appPage.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches),
  ]);
  if (motion[0] !== reduced || motion[1] !== reduced) {
    throw new Error(`reduced-motion flags mock=${motion[0]} app=${motion[1]} expected=${reduced}`);
  }
  await mockPage.evaluate(() => window.__cap.start());
  await appPage.evaluate(() => window.__shellCap.start());

  const shotOpts = { type: "jpeg", quality: 85, animations: "allow" };
  const firstPress = { mock: {}, app: {} };
  const motionLog = [];
  let frame = 0;
  const motionProbe = () => {
    let moving = false;
    for (const anim of document.getAnimations()) {
      const timing = anim.effect?.getComputedTiming();
      const end = timing ? timing.endTime : 0;
      const current = typeof anim.currentTime === "number" ? anim.currentTime : 0;
      if (typeof end === "number" && Number.isFinite(end) && current + 0.5 < end) {
        moving = true;
        break;
      }
    }
    const cursor = document.querySelector(".cursor");
    const win = document.querySelector(".win");
    const composer = document.querySelector("#composer, .composer");
    const styleOf = (el) =>
      el instanceof HTMLElement
        ? `${el.style.left}|${el.style.top}|${el.style.width}|${el.style.height}|${el.style.opacity}|${el.style.transform}`
        : "";
    return {
      moving,
      cursor: cursor instanceof HTMLElement ? `${cursor.style.transform}|${cursor.style.opacity}` : "",
      win: styleOf(win),
      composer: styleOf(composer),
    };
  };
  const notePress = async () => {
    const marks = await Promise.all([
      mockPage.evaluate(pressMark),
      appPage.evaluate(pressMark),
    ]);
    for (const [side, mark] of [["mock", marks[0]], ["app", marks[1]]]) {
      if (mark && firstPress[side][mark] == null) {
        firstPress[side][mark] = frame;
      }
    }
  };
  const paintFlush = (page) =>
    page.evaluate(
      () =>
        new Promise((resolve) => {
          requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined)));
        }),
    );
  const grab = async () => {
    await paintFlush(mockPage);
    await paintFlush(appPage);
    const id = String(frame).padStart(4, "0");
    await mockPage.screenshot({ ...shotOpts, path: path.join(frameDir, `mock-${id}.jpg`) });
    await appPage.screenshot({ ...shotOpts, path: path.join(frameDir, `app-${id}.jpg`) });
    const flags = await Promise.all([mockPage.evaluate(motionProbe), appPage.evaluate(motionProbe)]);
    motionLog.push({ mock: flags[0], app: flags[1] });
    await notePress();
    frame += 1;
  };
  await grab();
  for (let i = 0; i < 720; i += 1) {
    await mockPage.evaluate((dt) => window.__cap.step(dt), DT);
    await appPage.evaluate((dt) => window.__shellCap.step(dt), DT);
    await grab();
    const done = await mockPage.evaluate(() => window.__cap.done);
    const appDone = await appPage.evaluate(() => !window.__shellCap.running() && window.__shellCap.pending() === 0);
    if (done && appDone && frame > 60) {
      break;
    }
  }
  await wide.close();
  console.log(`captured ${frame} frames reduced=${reduced} presses=${JSON.stringify(firstPress)}`);
  if (frame < minFrames) {
    throw new Error(`sequence ended early at ${frame} frames`);
  }
  const shared = reduced ? ["companion"] : ["companion", "pill", "composer"];
  for (const mark of shared) {
    if (firstPress.mock[mark] == null || firstPress.mock[mark] !== firstPress.app[mark]) {
      throw new Error(`press ${mark} frames differ ${JSON.stringify(firstPress)}`);
    }
  }
  return { frame, motion: motionLog };
}

const browser = await launch();
try {
  if (!videoOnly) {
    const stills = await browser.newContext({
      viewport: { width: 1280, height: 800 },
      deviceScaleFactor: 2,
      timezoneId: "America/New_York",
    });
    const page = await stills.newPage();
    await page.goto(`${origin}/?shellStage=1`, { waitUntil: "networkidle" });
    await page.locator(".wordmark").waitFor();
    await page.waitForTimeout(1400);
    for (const form of ["full", "companion", "pill"]) {
      await page.evaluate((next) => window.__shellStage.snap(next), form);
      await park(page, form === "pill" ? "desk" : "title");
      await assertChrome(page, form);
      await shot(page, `${form}.png`);
    }
    const idle = await page.evaluate(() => document.getAnimations().filter((anim) => anim.playState === "running").length);
    if (idle !== 0) {
      throw new Error(`${idle} animations still running at rest`);
    }
    await stills.close();
  }

  const reduced = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    reducedMotion: "reduce",
  });
  const quiet = await reduced.newPage();
  await quiet.goto(`${origin}/?shellStage=1`, { waitUntil: "networkidle" });
  await quiet.locator(".wordmark").waitFor();
  const reducedReport = await quiet.evaluate(async () => {
    const before = performance.now();
    await window.__shellStage.morph("companion");
    const win = document.querySelector(".win");
    return {
      ms: performance.now() - before,
      transform: win ? getComputedStyle(win).transform : "",
      running: document.getAnimations().filter((anim) => anim.playState === "running").length,
      form: window.__shellStage.form(),
    };
  });
  if (reducedReport.form !== "companion" || reducedReport.transform !== "none" || reducedReport.running !== 0) {
    throw new Error(`reduced morph ${JSON.stringify(reducedReport)}`);
  }
  console.log(`reduced morph ${Math.round(reducedReport.ms)}ms`);
  await reduced.close();

  if (process.env.SKIP_FULL !== "1") {
    const fullCap = await capturePair(browser, { reduced: false, minFrames: 400 });
    const fullDup = await duplicateReport();
    const fullMotion = countMotionDuplicates(await frameHashes("mock-"), fullCap.motion, "mock");
    const fullVideo = path.join(outDir, "shell-vs-mock.mp4");
    await encode(fullVideo, "Mock", "App");
    await rm(frameDir, { recursive: true, force: true });
    const fullProbe = await readFrameCount(fullVideo);
    if (Number(fullProbe.nb_frames) !== fullDup.frames || fullProbe.avg_frame_rate !== "60/1") {
      throw new Error(`shell-vs-mock timeline ${JSON.stringify(fullProbe)} captured ${fullDup.frames}`);
    }
    if (fullMotion.motionDuplicates !== 0) {
      throw new Error(`shell-vs-mock moved without a new frame: ${JSON.stringify(fullMotion.hits)}`);
    }
    console.log(
      `wrote ${fullVideo} rest-duplicates=${fullMotion.restDuplicates} motion-duplicates=${fullMotion.motionDuplicates} composite=${fullDup.composite}`,
    );
  }

  const quietCap = await capturePair(browser, { reduced: true, minFrames: 300 });
  const quietDup = await duplicateReport();
  const quietMotion = countMotionDuplicates(await frameHashes("app-"), quietCap.motion, "app");
  const quietMockMotion = countMotionDuplicates(await frameHashes("mock-"), quietCap.motion, "mock");
  if (quietMockMotion.motionDuplicates !== 0) {
    throw new Error(`shell-reduced mock moved without a new frame: ${JSON.stringify(quietMockMotion.hits)}`);
  }
  const quietVideo = path.join(outDir, "shell-reduced.mp4");
  await encode(quietVideo, "Mock reduced", "App reduced");
  await rm(frameDir, { recursive: true, force: true });
  const quietProbe = await readFrameCount(quietVideo);
  if (Number(quietProbe.nb_frames) !== quietDup.frames || quietProbe.avg_frame_rate !== "60/1") {
    throw new Error(`shell-reduced timeline ${JSON.stringify(quietProbe)} captured ${quietDup.frames}`);
  }
  if (quietMotion.motionDuplicates !== 0) {
    throw new Error(`shell-reduced moved without a new frame: ${JSON.stringify(quietMotion.hits)}`);
  }
  console.log(
    `wrote ${quietVideo} rest-duplicates=${quietMotion.restDuplicates} motion-duplicates=${quietMotion.motionDuplicates} composite=${quietDup.composite}`,
  );
} finally {
  await browser.close();
  preview.kill("SIGTERM");
  mock.server.close();
}
