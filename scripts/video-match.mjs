/**
 * Frame-matched side-by-sides against the mock videos.
 * The app clock steps 16.667ms. Frame 0 is sequence time 0, the same
 * input the thread video starts on. Stamps read t+N ms fN on both sides.
 *
 * Usage: node scripts/video-match.mjs [origin]
 * REDUCED=1 captures reduced motion (files get a -reduced suffix). DPR (default 2)
 * sets the capture pixel ratio; the app pane is downsampled to 1440x900.
 * Expects the desktop preview. Writes
 *   docs/review/thread/thread-vs-video-1440.mp4
 *   docs/review/thread/card-vs-video-1440.mp4
 *   docs/review/thread/video-match.json
 */
import { createRequire } from "node:module";
import { mkdirSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { relative } from "node:path";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const origin = process.argv[2] ?? "http://127.0.0.1:4173";
const reviewDir = fileURLToPath(new URL("../docs/review/thread/", import.meta.url));
const repoDir = fileURLToPath(new URL("../", import.meta.url));
const refDir = fileURLToPath(new URL("../docs/reference/thread/", import.meta.url));
const FRAME_MS = 16.667;
const WIDTH = 1440;
const HEIGHT = 900;
const FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";
/** Card video frame 0 matches the thread video just before the card lifts. */
const CARD_ALIGN_MS = 8400;
/** The card video's hold press lands 3474ms after its frame 0 (full motion: holdStart 11874 - 8400). */
const CARD_HOLD_OFFSET_MS = 3474;
/** REDUCED=1 captures with prefers-reduced-motion: reduce. */
const REDUCED = process.env.REDUCED === "1";
/** Device pixel ratio of the app capture. The app pane is downsampled to the video's 1440x900. */
const DPR = Number(process.env.DPR ?? 2);
const SUFFIX = REDUCED ? "-reduced" : "";

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function run(cmd, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: ["ignore", "inherit", "inherit"] });
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${cmd} ${code}`))));
  });
}

function frameCount(file) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      "ffprobe",
      ["-v", "error", "-select_streams", "v:0", "-count_packets", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0", file],
      { stdio: ["ignore", "pipe", "inherit"] },
    );
    let out = "";
    child.stdout.on("data", (chunk) => {
      out += chunk.toString();
    });
    child.on("exit", (code) => {
      if (code !== 0) {
        reject(new Error(`ffprobe ${code}`));
        return;
      }
      resolve(Number(out.trim()));
    });
  });
}

function openPipe(path) {
  const child = spawn(
    "ffmpeg",
    [
      "-y",
      "-f",
      "image2pipe",
      "-framerate",
      "60",
      "-i",
      "-",
      "-an",
      "-c:v",
      "libx264",
      "-pix_fmt",
      "yuv420p",
      "-preset",
      "veryfast",
      "-crf",
      "16",
      path,
    ],
    { stdio: ["pipe", "ignore", "pipe"] },
  );
  let err = "";
  child.stderr.on("data", (chunk) => {
    err += chunk.toString();
  });
  const done = new Promise((resolve, reject) => {
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg pipe ${code}: ${err.slice(-400)}`))));
  });
  return { stdin: child.stdin, done };
}

async function writeFrame(stdin, buf) {
  if (!stdin.write(buf)) {
    await new Promise((resolve) => stdin.once("drain", resolve));
  }
}

async function shoot(cdp) {
  let last = new Error("screenshot failed");
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      const shot = await cdp.send("Page.captureScreenshot", { format: "jpeg", quality: 82 });
      return Buffer.from(shot.data, "base64");
    } catch (err) {
      last = err instanceof Error ? err : new Error(String(err));
      await sleep(40);
    }
  }
  throw last;
}

async function installAppClock(page) {
  await page.evaluate(() => {
    const win = window;
    let t = 0;
    let seq = 1;
    const epoch = Date.now();
    const timers = [];
    const intervals = new Set();
    let rafs = [];
    const origin = new WeakMap();
    win.setTimeout = (fn, ms, ...args) => {
      const id = ++seq;
      timers.push({
        id,
        at: t + Math.max(0, Number(ms) || 0),
        fn: () => {
          if (typeof fn === "function") {
            fn(...args);
          }
        },
      });
      return id;
    };
    win.clearTimeout = (id) => {
      const index = timers.findIndex((timer) => timer.id === id);
      if (index >= 0 && !intervals.has(id)) {
        timers.splice(index, 1);
      }
    };
    win.setInterval = (fn, ms, ...args) => {
      const id = ++seq;
      const period = Math.max(0, Number(ms) || 0);
      intervals.add(id);
      const arm = () => {
        timers.push({
          id,
          at: t + period,
          fn: () => {
            if (!intervals.has(id)) {
              return;
            }
            if (typeof fn === "function") {
              fn(...args);
            }
            if (intervals.has(id)) {
              arm();
            }
          },
        });
      };
      arm();
      return id;
    };
    win.clearInterval = (id) => {
      intervals.delete(id);
      for (let index = timers.length - 1; index >= 0; index -= 1) {
        if (timers[index].id === id) {
          timers.splice(index, 1);
        }
      }
    };
    win.requestAnimationFrame = (cb) => {
      const id = ++seq;
      rafs.push({ id, cb });
      return id;
    };
    win.cancelAnimationFrame = (id) => {
      rafs = rafs.filter((raf) => raf.id !== id);
    };
    win.performance.now = () => t;
    win.Date.now = () => epoch + Math.round(t);
    const drain = async () => {
      for (let turn = 0; turn < 16; turn += 1) {
        await Promise.resolve();
      }
    };
    const fireDue = async (limit) => {
      let fired = 0;
      while (fired < 800) {
        let best = -1;
        for (let index = 0; index < timers.length; index += 1) {
          if (timers[index].at <= limit && (best < 0 || timers[index].at < timers[best].at)) {
            best = index;
          }
        }
        if (best < 0) {
          return fired;
        }
        const timer = timers.splice(best, 1)[0];
        t = Math.max(t, timer.at);
        timer.fn();
        await drain();
        fired += 1;
      }
      throw new Error("capture clock stalled on timers");
    };
    const syncAnims = () => {
      for (const anim of win.document.getAnimations()) {
        if (anim.playState === "finished" || anim.playState === "idle") {
          continue;
        }
        if (!origin.has(anim)) {
          try {
            anim.pause();
          } catch {
            // Leave an unplayable animation for the next tick.
          }
          origin.set(anim, t);
        }
        const local = t - origin.get(anim);
        const timing = anim.effect ? anim.effect.getComputedTiming() : null;
        const end = timing ? timing.endTime : 0;
        try {
          if (Number.isFinite(end) && local >= end) {
            anim.finish();
          } else {
            anim.currentTime = local;
          }
        } catch {
          // Finished between the play-state check and the write.
        }
      }
    };
    win.__clock = {
      get now() {
        return t;
      },
      async step(dt) {
        const target = t + dt;
        await fireDue(target);
        t = target;
        const batch = rafs;
        rafs = [];
        for (const raf of batch) {
          raf.cb(t);
        }
        await drain();
        await fireDue(t);
        syncAnims();
      },
    };
  });
}

function stampFilter(title) {
  const time = "%{eif\\:trunc(n*16.667+0.5)\\:d}";
  return [
    "pad=iw:ih+32:0:32:color=0xF6F2EB",
    `drawtext=fontfile=${FONT}:text='${title}':x=16:y=7:fontsize=18:fontcolor=0x2B2723`,
    `drawtext=fontfile=${FONT}:text='t+${time} ms  f%{n}':x=w-tw-16:y=7:fontsize=18:fontcolor=0x2B2723`,
  ].join(",");
}

const appScale = `scale=${WIDTH}:${HEIGHT}:flags=lanczos`;
const appTitle = `App DPR ${DPR}${REDUCED ? " reduced" : ""}`;

async function stack(appPath, refPath, out, startFrame, count) {
  const appChain =
    startFrame > 0
      ? `[0:v]trim=start_frame=${startFrame}:end_frame=${startFrame + count},setpts=PTS-STARTPTS,${appScale},${stampFilter(appTitle)}[app]`
      : `[0:v]${appScale},${stampFilter(appTitle)}[app]`;
  const filter = [
    appChain,
    `[1:v]${stampFilter("Video")}[vid]`,
    "[vid][app]hstack=inputs=2",
  ].join(";");
  await run("ffmpeg", [
    "-y",
    "-i",
    appPath,
    "-i",
    refPath,
    "-filter_complex",
    filter,
    "-an",
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    "-preset",
    "veryfast",
    "-crf",
    "23",
    "-fps_mode",
    "passthrough",
    out,
  ]);
}

const threadFrames = await frameCount(`${refDir}thread.mp4`);
const cardFrames = await frameCount(`${refDir}approval-hold-undo-file.mp4`);
if (threadFrames < 1000 || cardFrames < 700) {
  throw new Error(`reference frame counts look wrong (${threadFrames}, ${cardFrames})`);
}

const browser = await chromium.launch({
  executablePath: "/usr/bin/google-chrome",
  args: ["--disable-dev-shm-usage"],
});
const context = await browser.newContext({
  viewport: { width: WIDTH, height: HEIGHT },
  deviceScaleFactor: DPR,
  timezoneId: "America/New_York",
  reducedMotion: REDUCED ? "reduce" : "no-preference",
});
const page = await context.newPage();
await page.goto(`${origin}/?capture=1`, { waitUntil: "networkidle" });
await page.waitForFunction(() => Boolean(window.__thread?.start));
await page.evaluate(() => document.fonts.ready);
await installAppClock(page);
const cdp = await context.newCDPSession(page);
await cdp.send("Emulation.setDeviceMetricsOverride", {
  width: WIDTH,
  height: HEIGHT,
  deviceScaleFactor: DPR,
  mobile: false,
});
await page.evaluate(() => {
  window.__thread.start();
});
await page.evaluate(() => window.__clock.step(0));

const appPath = `/tmp/thread-app-1440${SUFFIX}.mp4`;
const steps = [];
let duplicateSteps = 0;
let prevClock = null;
const pipe = openPipe(appPath);
let cardFrame = -1;
const started = Date.now();
try {
  for (let frame = 0; frame < threadFrames; frame += 1) {
    const clock = await page.evaluate(() => window.__clock.now);
    if (prevClock !== null) {
      const dt = clock - prevClock;
      steps.push(dt);
      if (Math.abs(dt - FRAME_MS) > 0.001) {
        duplicateSteps += 1;
      }
    }
    prevClock = clock;
    if (cardFrame < 0 && clock >= CARD_ALIGN_MS) {
      cardFrame = frame;
    }
    const buf = await shoot(cdp);
    await writeFrame(pipe.stdin, buf);
    if (frame + 1 < threadFrames) {
      await page.evaluate((dt) => window.__clock.step(dt), FRAME_MS);
    }
    if (frame > 0 && frame % 120 === 0) {
      const marks = await page.evaluate(() => ({ ...(window.__thread?.marks ?? {}) }));
      console.log(
        `f${frame} t${Math.round(clock)} cardFrame ${cardFrame} send ${marks.sendStart ?? "-"} hold ${marks.holdStart ?? "-"} ${((Date.now() - started) / 1000).toFixed(0)}s`,
      );
    }
  }
} catch (err) {
  pipe.stdin.destroy();
  await browser.close();
  throw err;
}
pipe.stdin.end();
const marks = await page.evaluate(() => ({ ...(window.__thread?.marks ?? {}), clock: window.__clock.now }));
await pipe.done;
await browser.close();

if (REDUCED && marks.holdStart != null) {
  // Reduced motion runs a shorter story. Align the card video on its hold press instead.
  cardFrame = Math.round((marks.holdStart - CARD_HOLD_OFFSET_MS) / FRAME_MS);
}
if (cardFrame < 0 || cardFrame + cardFrames > threadFrames) {
  throw new Error(`card slice ${cardFrame} + ${cardFrames} does not fit in ${threadFrames}`);
}

mkdirSync(reviewDir, { recursive: true });
const threadOut = `${reviewDir}thread-vs-video-1440${SUFFIX}.mp4`;
const cardOut = `${reviewDir}card-vs-video-1440${SUFFIX}.mp4`;
await stack(appPath, `${refDir}thread.mp4`, threadOut, 0, threadFrames);
await stack(appPath, `${refDir}approval-hold-undo-file.mp4`, cardOut, cardFrame, cardFrames);

const report = {
  frameMs: FRAME_MS,
  width: WIDTH,
  height: HEIGHT,
  threadFrames,
  cardFrames,
  cardFrame,
  cardAlignMs: CARD_ALIGN_MS,
  marks,
  dpr: DPR,
  reduced: REDUCED,
  crf: 23,
  clockStepMin: Math.min(...steps),
  clockStepMax: Math.max(...steps),
  duplicateSteps,
  threadOut: relative(repoDir, threadOut),
  cardOut: relative(repoDir, cardOut),
  threadOutFrames: await frameCount(threadOut),
  cardOutFrames: await frameCount(cardOut),
  videoFirstActionFrame: 88,
  sendStartMs: marks.sendStart ?? null,
};
if (duplicateSteps > 0) {
  throw new Error(`${duplicateSteps} clock steps were not ${FRAME_MS}ms`);
}
writeFileSync(`${reviewDir}video-match${SUFFIX}.json`, JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
