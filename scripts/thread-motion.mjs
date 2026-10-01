/**
 * Side-by-side 60fps proof.
 * Each output frame is one screenshot after both panes advance exactly 1/60s
 * on a stubbed clock (rAF, performance.now, Date.now, timers, and CSS animations).
 * Chrome's virtual-time policy is not used: pausing it stalls the compositor,
 * so captureScreenshot never returns.
 *
 * Frame 0 is the shared send at rest: both composers empty, both bubbles landed.
 * The label stamp is t+N ms fN from that frame, at a 16.667ms step.
 *
 * Usage: node scripts/thread-motion.mjs [origin]
 * Expects the desktop preview (default http://127.0.0.1:4173).
 * MARKS_ONLY=1 steps the clocks and writes motion-marks.json.
 * WIDTH=1280 or 1440 (default both). DPR is 2.
 */
import { createRequire } from "node:module";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const origin = process.argv[2] ?? "http://127.0.0.1:4173";
const reviewDir = fileURLToPath(new URL("../docs/review/thread/", import.meta.url));
const tokensPath = fileURLToPath(new URL("../apps/desktop/src/lib/styles/tokens.css", import.meta.url));
const fontDir = fileURLToPath(new URL("../apps/desktop/src/fonts/", import.meta.url));
const FRAME_MS = 16.667;
const MAX_FRAMES = Number(process.env.FRAMES ?? 70 * 60);
const limited = Boolean(process.env.FRAMES);

const fontAlias = {
  "Figtree-latin-wght.woff2": "figtree-latin-wght-normal.woff2",
  "Figtree-latin-ext-wght.woff2": "figtree-latin-ext-wght-normal.woff2",
  "JetBrainsMono-latin-wght.woff2": "jetbrains-mono-latin-wght-normal.woff2",
  "JetBrainsMono-latin-ext-wght.woff2": "jetbrains-mono-latin-ext-wght-normal.woff2",
};

const prototypeHtml = readFileSync(
  fileURLToPath(new URL("../docs/reference/thread/thread.html", import.meta.url)),
  "utf8",
).replaceAll('format("woff2")', 'format("woff2-variations")');

function compareDocument(width, height) {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>thread compare</title>
<style>
  html, body { margin: 0; background: #F6F2EB; overflow: hidden; }
  .row { display: flex; width: ${width * 2}px; }
  .col { width: ${width}px; }
  .label { height: 32px; display: flex; align-items: center; justify-content: space-between; padding: 0 16px;
    font: 600 18px/1 "DejaVu Sans", sans-serif; color: #2B2723; }
  iframe { width: ${width}px; height: ${height}px; border: 0; display: block; }
</style>
</head>
<body>
<div class="row">
  <div class="col">
    <div class="label"><span>Prototype</span><span class="stamp">t+0 ms  f0</span></div>
    <iframe id="proto" title="Prototype"></iframe>
  </div>
  <div class="col">
    <div class="label"><span>App</span><span class="stamp">t+0 ms  f0</span></div>
    <iframe id="app" title="App" src="/?capture=1"></iframe>
  </div>
</div>
<script>
  const reduced = new URLSearchParams(location.search).has("rm");
  document.getElementById("proto").src = "/ref/thread.html?capture=1" + (reduced ? "&rm=1" : "");
</script>
</body>
</html>`;
}

function jpegSize(buf) {
  let index = 2;
  while (index < buf.length - 8) {
    if (buf[index] !== 0xff) {
      break;
    }
    const marker = buf[index + 1];
    const length = buf.readUInt16BE(index + 2);
    if (marker === 0xc0 || marker === 0xc2) {
      return { height: buf.readUInt16BE(index + 5), width: buf.readUInt16BE(index + 7) };
    }
    index += 2 + length;
  }
  return { width: 2880, height: 932 };
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function runFfmpeg(args) {
  return new Promise((resolve, reject) => {
    const child = spawn("ffmpeg", args, { stdio: "inherit" });
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg ${code}`))));
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
      "14",
      path,
    ],
    { stdio: ["pipe", "ignore", "pipe"] },
  );
  let err = "";
  child.stderr.on("data", (chunk) => {
    err += chunk.toString();
  });
  const done = new Promise((resolve, reject) => {
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`ffmpeg pipe ${code}: ${err.slice(-500)}`));
    });
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
      const shot = await cdp.send("Page.captureScreenshot", { format: "jpeg", quality: 85 });
      return Buffer.from(shot.data, "base64");
    } catch (err) {
      last = err instanceof Error ? err : new Error(String(err));
      await sleep(50);
    }
  }
  throw last;
}

function pane(x, y, cropW, cropH, outW, outH) {
  return [`crop=${cropW}:${cropH}:${x}:${y}`, `scale=${outW}:${outH}`, "setpts=N/60/TB"].join(",");
}

function countDuplicateHashes(file) {
  return new Promise((resolve, reject) => {
    const child = spawn("ffmpeg", ["-i", file, "-f", "framemd5", "-"], { stdio: ["ignore", "pipe", "pipe"] });
    let buf = "";
    let prev = "";
    let duplicates = 0;
    let frames = 0;
    let run = 0;
    let longest = 1;
    child.stdout.on("data", (chunk) => {
      buf += chunk.toString();
      let index = buf.indexOf("\n");
      while (index >= 0) {
        const line = buf.slice(0, index).trim();
        buf = buf.slice(index + 1);
        if (line && !line.startsWith("#")) {
          const hash = line.split(",").at(-1) ?? "";
          if (frames > 0 && hash === prev) {
            duplicates += 1;
            run += 1;
            if (run > longest) {
              longest = run;
            }
          } else {
            run = 1;
          }
          prev = hash;
          frames += 1;
        }
        index = buf.indexOf("\n");
      }
    });
    let err = "";
    child.stderr.on("data", (chunk) => {
      err += chunk.toString();
    });
    child.on("exit", (code) => {
      if (code !== 0) {
        reject(new Error(`framemd5 ${code}: ${err.slice(-400)}`));
        return;
      }
      resolve({ duplicates, frames, longest });
    });
  });
}

async function installAppClock(frame) {
  await frame.evaluate(() => {
    const win = window;
    let t = 0;
    let seq = 1;
    const epoch = Date.now();
    /** @type {{ id: number, at: number, fn: () => void }[]} */
    const timers = [];
    const intervals = new Set();
    /** @type {{ id: number, cb: (time: number) => void }[]} */
    let rafs = [];
    const origin = new WeakMap();

    win.setTimeout = (fn, ms, ...args) => {
      const id = ++seq;
      const delay = Math.max(0, Number(ms) || 0);
      timers.push({
        id,
        at: t + delay,
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
            // Unplayable animations are left for the next tick.
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
          // The animation finished between the play-state check and the write.
        }
      }
    };

    win.__clock = {
      get now() {
        return t;
      },
      get pending() {
        return timers.length + rafs.length;
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

async function paint(page) {
  await page.evaluate(
    () =>
      new Promise((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
}

async function openCompare(browser, { reduced, width, height }) {
  const context = await browser.newContext({
    viewport: { width: width * 2, height: height + 32 },
    timezoneId: "America/New_York",
    deviceScaleFactor: 2,
    reducedMotion: reduced ? "reduce" : "no-preference",
  });
  const page = await context.newPage();
  await page.route("**/cmp.html*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/html",
      body: compareDocument(width, height),
    }),
  );
  await page.route("**/ref/thread.html*", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: prototypeHtml }),
  );
  await page.route("**/ref/tokens.css*", (route) =>
    route.fulfill({ status: 200, contentType: "text/css", path: tokensPath }),
  );
  await page.route("**/ref/fonts/*", (route) => {
    const name = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    const file = fontAlias[name];
    if (!file) {
      return route.abort();
    }
    return route.fulfill({ status: 200, contentType: "font/woff2", path: join(fontDir, file) });
  });

  const query = `?w=${width}&h=${height}${reduced ? "&rm=1" : ""}`;
  await page.goto(`${origin}/cmp.html${query}`, { waitUntil: "networkidle" });
  await page.frameLocator("#proto").locator(".brand").waitFor();
  await page.frameLocator("#app").locator(".wordmark").waitFor();
  await page.waitForFunction(() => {
    const proto = document.getElementById("proto").contentWindow;
    const app = document.getElementById("app").contentWindow;
    return Boolean(proto?.__cap?.start && app?.__thread?.start);
  });
  await page.evaluate(() =>
    Promise.all([
      document.getElementById("proto").contentWindow.document.fonts.ready,
      document.getElementById("app").contentWindow.document.fonts.ready,
    ]),
  );

  const appFrame = page.frames().find((frame) => {
    const url = new URL(frame.url());
    return url.pathname === "/" && url.searchParams.has("capture");
  });
  if (!appFrame) {
    throw new Error(`app frame missing (${page.frames().map((frame) => frame.url()).join(", ")})`);
  }
  await installAppClock(appFrame);
  const motion = await page.evaluate(() => {
    const proto = document.getElementById("proto").contentWindow;
    const app = document.getElementById("app").contentWindow;
    return {
      protoReduced: proto.matchMedia("(prefers-reduced-motion: reduce)").matches,
      appReduced: app.matchMedia("(prefers-reduced-motion: reduce)").matches,
      protoRm: proto.document.documentElement.classList.contains("rm"),
      clock: Boolean(app.__clock),
    };
  });
  if (!motion.clock) {
    throw new Error("app capture clock was not installed");
  }
  if (reduced && (!motion.protoReduced || !motion.appReduced || !motion.protoRm)) {
    throw new Error(`reduced motion did not apply (${JSON.stringify(motion)})`);
  }

  await page.evaluate(() => {
    const proto = document.getElementById("proto").contentWindow;
    const app = document.getElementById("app").contentWindow;
    proto.__cap.start();
    app.__thread.start();
  });
  return { context, page };
}

async function stepBoth(page) {
  return page.evaluate(async (dt) => {
    const proto = document.getElementById("proto").contentWindow;
    const app = document.getElementById("app").contentWindow;
    await proto.__cap.step(dt);
    await app.__clock.step(dt);
    const protoMarks = { ...(proto.__thread?.marks ?? {}) };
    const appMarks = { ...(app.__thread?.marks ?? {}) };
    const protoField = proto.document.querySelector("#msg");
    const appField = app.document.querySelector("textarea");
    const landed = "Pushed. Review the handoff fix";
    const appLanded = [...app.document.querySelectorAll(".me")].some((el) =>
      (el.textContent || "").includes(landed),
    );
    const transforms = [];
    if (app.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      for (const el of app.document.querySelectorAll("body *")) {
        for (const anim of el.getAnimations()) {
          const effect = anim.effect;
          const frames = effect && effect.getKeyframes ? effect.getKeyframes() : [];
          const text = JSON.stringify(frames);
          if (text.includes("transform") || text.includes("translate") || text.includes("scale")) {
            transforms.push(String(el.className).slice(0, 80));
          }
        }
      }
    }
    return {
      protoRest:
        protoMarks.sendLanded != null &&
        protoField != null &&
        protoField.value === "" &&
        !proto.document.querySelector(".flyer"),
      appRest: appMarks.sendLanded != null && appField != null && appField.value === "" && appLanded,
      protoRunning: Boolean(proto.__thread?.running),
      appRunning: Boolean(app.__thread?.running),
      protoDone: Boolean(proto.__cap?.done),
      clock: app.__clock.now,
      beat: proto.document.getElementById("beat")?.textContent ?? "",
      protoMarks,
      appMarks,
      transforms: [...new Set(transforms)].slice(0, 12),
      paper: app.document.querySelector("article.card.paper")?.getBoundingClientRect().height ?? 0,
      ask: [...app.document.querySelectorAll("article.card")].map((el) => ({
        filed: el.dataset.filed || "",
        h: Math.round(el.getBoundingClientRect().height),
        cls: String(el.className).slice(0, 60),
        quiet: (el.querySelector(".quiet")?.textContent || "").slice(0, 70),
      })),
    };
  }, FRAME_MS);
}

async function stamp(page, frame, ms) {
  await page.evaluate(
    ({ frame, ms }) => {
      for (const node of document.querySelectorAll(".stamp")) {
        node.textContent = `t+${ms} ms  f${frame}`;
      }
    },
    { frame, ms },
  );
}

async function capture(browser, { reduced, width, height, out, shootFrames }) {
  const temp = `/tmp/thread-raw-${width}-${reduced ? "rm" : "full"}.mp4`;
  rmSync(temp, { force: true });
  const { context, page } = await openCompare(browser, { reduced, width, height });
  const cdp = await context.newCDPSession(page);
  const label = `${reduced ? "reduced" : "full"} ${width}`;
  let pipe = null;
  let captured = 0;
  let size = { width: width * 4, height: (height + 32) * 2 };
  const started = Date.now();
  let restAt = -1;
  let virtual = 0;
  const dirty = new Set();
  let last = null;
  try {
    for (let frame = 0; frame < MAX_FRAMES; frame += 1) {
      const status = await stepBoth(page);
      last = status;
      for (const name of status.transforms) {
        dirty.add(name);
      }
      const rested = status.protoRest && status.appRest;
      if (restAt < 0 && rested) {
        restAt = frame;
        virtual = 0;
      }
      if (restAt >= 0 && shootFrames) {
        await stamp(page, captured, Math.round(virtual));
        await paint(page);
        if (!pipe) {
          pipe = openPipe(temp);
        }
        const buf = await shoot(cdp);
        if (captured === 0) {
          size = jpegSize(buf);
        }
        await writeFrame(pipe.stdin, buf);
        virtual += FRAME_MS;
        captured += 1;
      }
      if (frame > 0 && frame % 120 === 0) {
        const elapsed = ((Date.now() - started) / 1000).toFixed(1);
        console.log(
          `${label} f${frame} v${Math.round(status.clock)} rest ${restAt} out ${captured} run ${status.appRunning}/${status.protoRunning} beat ${status.beat} ${elapsed}s`,
        );
      }
      const bothDone = !status.protoRunning && !status.appRunning && status.protoDone;
      if (!limited && restAt >= 0 && bothDone) {
        break;
      }
    }
  } catch (err) {
    pipe?.stdin.destroy();
    await context.close();
    throw new Error(`${label} frame ${captured}: ${err instanceof Error ? err.message : err}`);
  }
  if (pipe) {
    pipe.stdin.end();
  }
  const report = {
    width,
    height,
    reduced,
    dpr: 2,
    frameMs: FRAME_MS,
    restFrame: restAt,
    captured,
    clock: last?.clock ?? 0,
    protoMarks: last?.protoMarks ?? {},
    appMarks: last?.appMarks ?? {},
    paper: last?.paper ?? 0,
    ask: last?.ask ?? [],
    reducedTransforms: [...dirty],
  };
  await context.close();
  if (pipe) {
    await pipe.done;
  }
  if (restAt < 0) {
    throw new Error(`${label} never reached a shared rest frame`);
  }
  if (!shootFrames) {
    console.log(`${label} marks rest f${restAt} clock ${Math.round(report.clock)}`);
    return report;
  }
  const cssW = width * 2;
  const cssH = height + 32;
  const scaleX = size.width / cssW;
  const scaleY = size.height / cssH;
  const cropW = Math.round(width * scaleX);
  const cropH = Math.round(cssH * scaleY);
  const cropX = Math.round(width * scaleX);
  const filter = [
    "[0:v]split[a][b]",
    `[a]${pane(0, 0, cropW, cropH, width, cssH)}[left]`,
    `[b]${pane(cropX, 0, cropW, cropH, width, cssH)}[right]`,
    "[left][right]hstack=inputs=2",
  ].join(";");
  await runFfmpeg([
    "-y",
    "-i",
    temp,
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
    "18",
    "-fps_mode",
    "passthrough",
    "-r",
    "60",
    out,
  ]);
  rmSync(temp, { force: true });
  const hashes = await countDuplicateHashes(out);
  report.frames = hashes.frames;
  report.duplicateFrames = hashes.duplicates;
  report.stillRun = hashes.longest ?? hashes.duplicates;
  console.log(`${out} frames ${hashes.frames} duplicate ${hashes.duplicates} paper ${report.paper}`);
  if (hashes.frames !== captured) {
    throw new Error(`${out} encoded ${hashes.frames} frames, expected ${captured}`);
  }
  console.log(`wrote ${out}`);
  return report;
}

const widths = (process.env.WIDTH ?? "1280,1440")
  .split(",")
  .map((value) => Number(value.trim()))
  .filter((value) => value === 1280 || value === 1440);
const heights = { 1280: 800, 1440: 900 };
const marksOnly = process.env.MARKS_ONLY === "1";

const browser = await chromium.launch({
  channel: process.env.FONT_PROOF_CHANNEL || "chrome",
  args: ["--disable-dev-shm-usage"],
});
const reports = [];
try {
  for (const width of widths) {
    const height = heights[width];
    if (process.env.SKIP_FULL !== "1") {
      reports.push(
        await capture(browser, {
          reduced: false,
          width,
          height,
          shootFrames: !marksOnly && !limited,
          out: `${reviewDir}thread-vs-prototype-${width}.mp4`,
        }),
      );
    }
    if (!limited) {
      reports.push(
        await capture(browser, {
          reduced: true,
          width,
          height,
          shootFrames: !marksOnly,
          out: `${reviewDir}thread-reduced-${width}.mp4`,
        }),
      );
    }
  }
} finally {
  await browser.close();
}
mkdirSync(reviewDir, { recursive: true });
writeFileSync(join(reviewDir, "motion-marks.json"), JSON.stringify(reports, null, 2));
console.log(`wrote ${join(reviewDir, "motion-marks.json")}`);
