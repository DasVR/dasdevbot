/**
 * The scripted stage sequence against UI Designer's 8b98fae findings:
 * - UID 2: Builder's roster row reads "waiting on you", on one line, from the
 *   same tick as the count bump (mock 4a:1324, video frame 282), and Deployer
 *   moves up with it. Before that it is the video's two-line running sub.
 * - UID 3: the pill press aims at the centre of the whole "N waiting" pill
 *   (mock centerOf(.wt)), not the numeral.
 * - UID 4: the background window shows the thread behind a blur(3px) frost.
 * Runs under ?shellCapture, stepping the virtual clock at 60fps.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import net from "node:net";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));

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

function launchBrowser() {
  return chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
}

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;

const preview = spawn(process.execPath, [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: desktop,
  stdio: ["ignore", "ignore", "pipe"],
});

async function waitForHttp(url) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
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

async function main(browser) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(`${origin}/?shellCapture=1`, { waitUntil: "load" });
  await page.waitForFunction(() => typeof window.__shellCap?.start === "function");
  await page.evaluate(() => document.fonts.ready);

  const bg = await page.evaluate(() => {
    const img = document.querySelector(".other .bgthread");
    const frost = document.querySelector(".other .frost");
    return {
      loaded: img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0,
      filter: frost ? getComputedStyle(frost).backdropFilter : "",
    };
  });
  if (!bg.loaded || !/blur\(3px\)/.test(bg.filter)) {
    throw new Error(`UID 4: background window is not the frosted thread ${JSON.stringify(bg)}`);
  }

  const read = () =>
    page.evaluate(() => {
      const row = (id) => [...document.querySelectorAll(".roster .rrow")].find((el) => el.querySelector("b")?.textContent === id);
      const builder = row("Builder");
      const deployer = row("Deployer");
      const sub = builder?.querySelector(".sub");
      const count = document.querySelector(".cl.pill .wt .n");
      const style = count ? getComputedStyle(count) : null;
      const cursor = document.querySelector(".cursor");
      const m = cursor ? /translate\(([-\d.]+)px, ([-\d.]+)px\)/.exec(cursor.style.transform) : null;
      const wt = document.querySelector(".cl.pill .wt")?.getBoundingClientRect();
      return {
        t: window.__shellCap.now(),
        // textContent: in the pill the window is visibility:hidden, which innerText skips.
        sub: (sub?.textContent ?? "").trim(),
        subLines: sub ? Math.round(sub.getBoundingClientRect().height / parseFloat(getComputedStyle(sub).lineHeight)) : 0,
        deployerTop: deployer?.getBoundingClientRect().top ?? 0,
        rolling: Boolean(count && count.getAnimations().length > 0) || (style ? style.opacity !== "1" : false),
        press: document.querySelector(".composer")?.classList.contains("is-press") ?? false,
        cursor: m ? { x: Number(m[1]) + 3, y: Number(m[2]) + 2 } : null,
        wt: wt ? { x: wt.left + wt.width / 2, y: wt.top + wt.height / 2 } : null,
      };
    });

  const first = await read();
  if (first.sub.replace(/\s+/g, " ") !== "2m14s · 208 / 8000 tok" || first.subLines !== 2) {
    throw new Error(`Builder does not start on the video's two-line running sub ${JSON.stringify(first)}`);
  }
  await page.evaluate(() => window.__shellCap.start());
  let frame = 0;
  let flipFrame = -1;
  let rollFrame = -1;
  let pressAt = null;
  let before = first.deployerTop;
  let after = null;
  for (; frame < 900; frame += 1) {
    await page.evaluate(() => window.__shellCap.step(1000 / 60));
    const now = await read();
    if (rollFrame < 0 && now.rolling) {
      rollFrame = frame;
    }
    if (flipFrame < 0 && now.sub === "waiting on you") {
      flipFrame = frame;
      after = now;
    }
    if (!pressAt && now.press && now.cursor && now.wt) {
      pressAt = now;
    }
    const done = await page.evaluate(() => !window.__shellCap.running() && window.__shellCap.pending() === 0);
    if (done) {
      break;
    }
  }
  if (flipFrame < 0 || rollFrame < 0 || Math.abs(flipFrame - rollFrame) > 0) {
    throw new Error(`UID 2: Builder flipped on frame ${flipFrame}, the count bumped on frame ${rollFrame}`);
  }
  if (after.subLines !== 1) {
    throw new Error(`UID 2: Builder's "waiting on you" is ${after.subLines} lines`);
  }
  await page.evaluate(() => window.__shellStage?.snap?.("full"));
  const end = await read();
  if (!(end.deployerTop < before - 8)) {
    throw new Error(`UID 2: Deployer did not move up with Builder's one-line row (${before} -> ${end.deployerTop})`);
  }
  if (!pressAt) {
    throw new Error("UID 3: never saw the pill press");
  }
  const miss = Math.hypot(pressAt.cursor.x - pressAt.wt.x, pressAt.cursor.y - pressAt.wt.y);
  if (miss > 1.5) {
    throw new Error(`UID 3: the pill press is ${miss.toFixed(1)}px off the "N waiting" centre`);
  }
  console.log(
    `stage-sequence-smoke: flip=count bump frame ${flipFrame}, Deployer ${before.toFixed(1)} -> ${end.deployerTop.toFixed(1)}px, pill aim off ${miss.toFixed(2)}px`,
  );
  await page.close();
}

const browser = await launchBrowser();
try {
  await waitForHttp(origin);
  await main(browser);
  console.log("stage-sequence-smoke: pass");
} finally {
  await browser.close();
  preview.kill();
}
