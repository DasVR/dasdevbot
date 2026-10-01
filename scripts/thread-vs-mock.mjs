/**
 * Frame-matched side-by-side: the app alone vs two mock videos.
 *
 * The app runs in a 1440x900 viewport at DPR 2 on the same stubbed clock as
 * thread-motion.mjs. Every captured frame is exactly one 16.667ms clock step
 * (logged per frame). The mock is analyzed for its first-action frame by a
 * downscaled gray pixel diff against mock frame 0; the app side uses its own
 * __thread.marks. Both are trimmed so the first action is output frame 0.
 * A lead-in of LEAD frames is kept and labeled with negative indices.
 *
 * Usage: node scripts/thread-vs-mock.mjs [origin]
 * --mock-only prints the mock alignment and exits.
 */
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  FRAME_MS,
  countDuplicateHashes,
  installAppClock,
  openPipe,
  paint,
  runFfmpeg,
  shoot,
  writeFrame,
} from "./thread-motion.mjs";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const args = process.argv.slice(2);
const origin = args.find((arg) => !arg.startsWith("--")) ?? "http://127.0.0.1:4173";
const mockOnly = args.includes("--mock-only");
const reviewDir = fileURLToPath(new URL("../docs/review/thread/", import.meta.url));
const FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
const LEAD = 30;
const W = 1440;
const H = 900;
const MAX_FRAMES = 60 * 60;

const mocks = {
  thread: {
    file: "/workspace/dasdevbot-look/thread/thread.mp4",
    // The mock's first change is the first typed character; the app marks it as typeStart.
    appMark: "typeStart",
  },
  approval: {
    file: "/workspace/vid/approval-hold-undo-file.mp4",
    // The mock opens on the risky step and card lift; the hold press is the first change
    // in the Approve button after the landed card sits still. The app marks it as holdStart.
    appMark: "holdStart",
    afterStill: 30,
    region: [412, 690, 232, 36],
  },
};

/** First mock frame that differs from frame 0 by more than a few pixels (gray 360x225, |d|>12). */
function mockFirstAction(file, afterStill = 0, region = null) {
  const [w, h] = region ? [region[2], region[3]] : [360, 225];
  const vf = region
    ? `crop=${region[2]}:${region[3]}:${region[0]}:${region[1]},format=gray`
    : `scale=${w}:${h}:flags=area,format=gray`;
  const run = spawnSync("ffmpeg", ["-v", "error", "-i", file, "-vf", vf, "-f", "rawvideo", "-"], {
    maxBuffer: 1 << 30,
  });
  const area = region ? `region ${region.join(",")} at 1x` : `${w}x${h}`;
  const size = w * h;
  const total = run.stdout.length / size;
  const count = (a, b) => {
    let changed = 0;
    for (let p = 0; p < size; p += 1) {
      if (Math.abs(a[p] - b[p]) > 12) {
        changed += 1;
      }
    }
    return changed;
  };
  const at = (index) => run.stdout.subarray(index * size, (index + 1) * size);
  const diffs = [];
  const steps = [0];
  let first = -1;
  for (let index = 0; index < total; index += 1) {
    diffs.push(count(at(index), at(0)));
    if (index > 0) {
      steps.push(count(at(index), at(index - 1)));
    }
    if (first < 0 && diffs[index] >= 8) {
      first = index;
    }
  }
  if (!afterStill) {
    return { total, action: first, firstChange: first, threshold: `gray ${area}, |d|>12, >=8 px vs frame 0`, diffs };
  }
  // First change after a still run of afterStill frames that follows the first change.
  let still = 0;
  let action = -1;
  for (let index = first + 1; index < total; index += 1) {
    if (steps[index] < 8) {
      still += 1;
    } else if (still >= afterStill) {
      action = index;
      break;
    } else {
      still = 0;
    }
  }
  return {
    total,
    action,
    firstChange: first,
    threshold: `gray ${area}, |d|>12, >=8 px vs previous frame, after a ${afterStill}-frame still run`,
    diffs: steps,
  };
}

function appDocument() {
  return `<!doctype html>
<html><head><meta charset="utf-8"><style>
  html, body { margin: 0; overflow: hidden; background: #F6F2EB; }
  iframe { width: ${W}px; height: ${H}px; border: 0; display: block; }
</style></head>
<body><iframe id="app" title="App" src="/?capture=1"></iframe></body></html>`;
}

/** Runs the whole story once and keeps one JPEG per clock step under dir. */
async function captureApp(browser, reduced, dir) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const context = await browser.newContext({
    viewport: { width: W, height: H },
    timezoneId: "America/New_York",
    deviceScaleFactor: 2,
    reducedMotion: reduced ? "reduce" : "no-preference",
  });
  const page = await context.newPage();
  await page.route("**/vs.html*", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: appDocument() }),
  );
  await page.goto(`${origin}/vs.html${reduced ? "?rm=1" : ""}`, { waitUntil: "networkidle" });
  await page.frameLocator("#app").locator(".wordmark").waitFor();
  await page.waitForFunction(() => Boolean(document.getElementById("app").contentWindow?.__thread?.start));
  await page.evaluate(() => document.getElementById("app").contentWindow.document.fonts.ready);
  const appFrame = page.frames().find((frame) => new URL(frame.url()).searchParams.has("capture"));
  if (!appFrame) {
    throw new Error("app frame missing");
  }
  await installAppClock(appFrame);
  const appReduced = await appFrame.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  if (appReduced !== reduced) {
    throw new Error(`reduced motion mismatch: ${appReduced}`);
  }
  await appFrame.evaluate(() => window.__thread.start());
  const cdp = await context.newCDPSession(page);
  const clockLog = [];
  const hashes = [];
  const dirty = new Set();
  let marks = {};
  const started = Date.now();
  for (let index = 0; index < MAX_FRAMES; index += 1) {
    const status = await appFrame.evaluate(async (dt) => {
      const before = window.__clock.now;
      await window.__clock.step(dt);
      const after = window.__clock.now;
      const transforms = [];
      if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
        for (const el of document.querySelectorAll("body *")) {
          for (const anim of el.getAnimations()) {
            const text = JSON.stringify(anim.effect?.getKeyframes ? anim.effect.getKeyframes() : []);
            if (text.includes("transform") || text.includes("translate") || text.includes("scale")) {
              transforms.push(String(el.className).slice(0, 80));
            }
          }
        }
      }
      return {
        before,
        after,
        running: Boolean(window.__thread.running),
        marks: { ...window.__thread.marks },
        transforms,
      };
    }, FRAME_MS);
    for (const name of status.transforms) {
      dirty.add(name);
    }
    marks = status.marks;
    await paint(page);
    const buf = await shoot(cdp);
    writeFileSync(join(dir, `${String(index).padStart(5, "0")}.jpg`), buf);
    hashes.push(createHash("md5").update(buf).digest("hex"));
    clockLog.push({ frame: index, before: status.before, after: status.after, d: status.after - status.before });
    if (index % 240 === 0) {
      console.log(`${reduced ? "reduced" : "full"} f${index} t${Math.round(status.after)} ${((Date.now() - started) / 1000).toFixed(0)}s`);
    }
    if (!status.running && marks.end != null) {
      break;
    }
  }
  await context.close();
  return { dir, clockLog, hashes, marks, reducedTransforms: [...dirty] };
}

/** App frame index whose clock time first reaches the mark (frame i shows t = (i+1)*FRAME_MS). */
function frameForMark(clockLog, ms) {
  const index = clockLog.findIndex((entry) => entry.after >= ms);
  return index < 0 ? clockLog.length - 1 : index;
}

function stampExpr(side, offset, lead) {
  // n is the output frame index; output frame `lead` is the first action.
  const rel = `(n-${lead})`;
  return [
    `drawtext=fontfile=${FONT}:fontsize=40:fontcolor=0x2B2723:x=24:y=12:text='${side}'`,
    `drawtext=fontfile=${FONT}:fontsize=40:fontcolor=0x2B2723:x=w-tw-24:y=12:` +
      `text='t+%{eif\\:${rel}*16.667\\:d} ms  f%{eif\\:${rel}\\:d}  src %{eif\\:n+${offset}\\:d}'`,
  ].join(",");
}

async function compose({ name, mock, mockAction, app, appAction, reduced }) {
  const out = join(reviewDir, `${name}${reduced ? "-reduced" : ""}.mp4`);
  const lead = Math.min(LEAD, mockAction, appAction);
  const mockStart = mockAction - lead;
  const appStart = appAction - lead;
  const frames = Math.min(mock.total - mockStart, app.clockLog.length - appStart);
  const appLabel = reduced ? "App DPR 2 · reduced motion" : "App DPR 2";
  const filter = [
    `[0:v]trim=start_frame=${mockStart}:end_frame=${mockStart + frames},setpts=N/60/TB,` +
      `scale=2880:1800:flags=neighbor,pad=2880:1864:0:64:color=0xF6F2EB,` +
      `${stampExpr("Mock (1x, upscaled 2x)", mockStart, lead)}[l]`,
    // Screenshots are full-range JPEG; convert to TV range so both panes match the mock's levels.
    `[1:v]trim=end_frame=${frames},setpts=N/60/TB,scale=2880:1800:in_range=full:out_range=tv,format=yuv420p,` +
      `pad=2880:1864:0:64:color=0xF6F2EB,` +
      `${stampExpr(appLabel, appStart, lead)}[r]`,
    "[l][r]hstack=inputs=2,format=yuv420p[v]",
  ].join(";");
  await runFfmpeg([
    "-y",
    "-v",
    "error",
    "-i",
    mock.file,
    "-framerate",
    "60",
    "-start_number",
    String(appStart),
    "-i",
    join(app.dir, "%05d.jpg"),
    "-filter_complex",
    filter,
    "-map",
    "[v]",
    "-an",
    "-c:v",
    "libx264",
    "-preset",
    "veryfast",
    "-crf",
    "24",
    "-pix_fmt",
    "yuv420p",
    "-color_range",
    "tv",
    "-r",
    "60",
    "-fps_mode",
    "cfr",
    "-frames:v",
    String(frames),
    out,
  ]);
  const used = app.clockLog.slice(appStart, appStart + frames);
  const steps = used.map((entry) => entry.d);
  const badSteps = steps.filter((d) => Math.abs(d - FRAME_MS) >= 0.001).length;
  const appHashes = app.hashes.slice(appStart, appStart + frames);
  let identical = 0;
  for (let index = 1; index < appHashes.length; index += 1) {
    if (appHashes[index] === appHashes[index - 1]) {
      identical += 1;
    }
  }
  const encoded = await countDuplicateHashes(out);
  if (encoded.frames !== frames) {
    throw new Error(`${out} encoded ${encoded.frames} frames, expected ${frames}`);
  }
  if (badSteps !== 0) {
    throw new Error(`${out} has ${badSteps} frames that are not one clock step`);
  }
  console.log(`wrote ${out} frames ${frames}`);
  return {
    file: out.slice(out.indexOf("docs/")),
    reduced,
    leadIn: lead,
    frames,
    mockFirstActionFrame: mockAction,
    mockStartFrame: mockStart,
    appFirstActionFrame: appAction,
    appStartFrame: appStart,
    appMark: mocks[name.split("-")[0]].appMark,
    appMarkMs: app.marks[mocks[name.split("-")[0]].appMark],
    clockStepMin: Math.min(...steps),
    clockStepMax: Math.max(...steps),
    duplicateSteps: badSteps,
    encodedDuplicateFrames: encoded.duplicates,
    appConsecutiveIdenticalFrames: identical,
    reducedTransforms: reduced ? app.reducedTransforms : null,
    bytes: statSync(out).size,
  };
}

const mockInfo = {};
for (const [key, mock] of Object.entries(mocks)) {
  const info = mockFirstAction(mock.file, mock.afterStill, mock.region);
  mockInfo[key] = {
    ...mock,
    total: info.total,
    action: info.action,
    firstChange: info.firstChange,
    threshold: info.threshold,
  };
  console.log(`${key} mock frames ${info.total} first change f${info.firstChange} action f${info.action}`);
  console.log(`  diffs ${info.diffs.slice(Math.max(0, info.action - 3), info.action + 12).join(" ")}`);
}
if (mockOnly) {
  process.exit(0);
}

const browser = await chromium.launch({
  channel: process.env.FONT_PROOF_CHANNEL || "chrome",
  args: ["--disable-dev-shm-usage"],
});
const clips = [];
const runs = {};
try {
  for (const reduced of [false, true]) {
    const dir = `/tmp/thread-vs-mock-${reduced ? "rm" : "full"}`;
    const app = await captureApp(browser, reduced, dir);
    runs[reduced ? "reduced" : "full"] = {
      frames: app.clockLog.length,
      marks: app.marks,
      clockLog: app.clockLog.map((entry) => Number(entry.d.toFixed(6))),
    };
    for (const name of ["thread", "approval"]) {
      const mock = mockInfo[name];
      const markMs = app.marks[mock.appMark];
      if (markMs == null) {
        throw new Error(`app mark ${mock.appMark} missing`);
      }
      clips.push(
        await compose({
          name: `${name}-vs-mock`,
          mock,
          mockAction: mock.action,
          app,
          appAction: frameForMark(app.clockLog, markMs),
          reduced,
        }),
      );
    }
    rmSync(dir, { recursive: true, force: true });
  }
} finally {
  await browser.close();
}

writeFileSync(
  join(reviewDir, "vs-mock.json"),
  JSON.stringify(
    {
      frameMs: FRAME_MS,
      dpr: 2,
      viewport: [W, H],
      alignment:
        `Each clip keeps a lead-in of min(${LEAD}, mock action, app action) frames (leadIn per clip), stamped with ` +
        "negative f. Output frame leadIn is the first action on both sides; stamp fN = action + N*16.667ms. " +
        "'src' is the source frame index (mock video frame / app clock step).",
      mocks: Object.fromEntries(
        Object.entries(mockInfo).map(([key, info]) => [
          key,
          {
            file: info.file,
            frames: info.total,
            firstChangeFrame: info.firstChange,
            firstActionFrame: info.action,
            detector: info.threshold,
            appMark: info.appMark,
          },
        ]),
      ),
      clips,
      runs,
      notes: [
        "App frame i shows the clock at (i+1)*16.667ms from story start; the app first-action frame is the first step whose clock reaches the mark.",
        "Thread mock: first change vs frame 0 is the first typed character, aligned to the app's typeStart mark. The mock action is f17, so the lead-in is 17 frames, not 30.",
        "Approval mock: its first whole-frame change (f17) is the risky step and scroll, not the hold. The hold press is detected in the Approve button region as the first change after a 30-frame still run, and aligned to the app's holdStart mark. The app story then drives holdApprove (Ctrl+Enter keydown until Hello appears, keyup, Hello click after 600ms).",
        "Clip length is the shorter of the mock's remaining frames and the app story's remaining frames after alignment.",
        "Mocks have no reduced-motion version; reduced clips pair the reduced app with the same mock.",
      ],
    },
    null,
    2,
  ),
);
console.log(`wrote ${join(reviewDir, "vs-mock.json")}`);
