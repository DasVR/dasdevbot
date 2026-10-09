/**
 * Native resize lag. A Tauri window applies set_shell_bounds a little later
 * than the webview asks for it. With 0, 40 and 120ms of lag the layout must
 * settle on the native window's size (no stale 1360x828 inside a 400x790
 * companion), and the native composer must use the opaque fallback fill,
 * since a transparent window with no acrylic gives the blur nothing to sample.
 * The Tauri IPC is a stub; set_shell_bounds resizes the page after `lag` ms.
 *
 * UID 1 (8b98fae): the composer must also stay inside the window on every
 * frame of every morph, not just once it settles. A rAF sampler records the
 * worst overflow while each morph runs (it was 948 / 738 / 30 px).
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

// Playwright's headless shell, not the Chrome channel: headless Chrome drops
// rAF to ~1fps in a 400x52 (pill) viewport (measured 1.5 vs 60.5 fps on the
// same page), which no per-frame assertion survives. WebView2 does not.
function launchBrowser() {
  return chromium.launch();
}

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;

const snapshot = {
  protocol: 1,
  role: "executor",
  node: "lag-smoke",
  provider: "mock",
  provider_detail: "mock provider (demo, no network)",
  sync: "stub",
  endpoint_id: null,
  agents: [],
  approvals: [],
  ledger: [],
  events: [],
};

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

// F1: the old pill->full jumped 746px in one frame; a real morph moves under
// 150px a frame at 60fps over the 520ms stage (measured max 115px, the first
// ease-out frame of full->companion).
const MAX_JUMP = 150;

async function run(browser, lag) {
  const context = await browser.newContext({ viewport: { width: 1360, height: 828 } });
  const page = await context.newPage();
  let last = Date.now();
  await page.exposeFunction("__resize", async (w, h) => {
    last = Date.now();
    await page.setViewportSize({ width: Math.max(1, Math.round(w)), height: Math.max(1, Math.round(h)) });
  });
  await page.addInitScript(
    ({ snapshot, lag }) => {
      const targets = {
        full: { x: 40, y: 52, width: 1360, height: 828, radius: 14, glass: false, alwaysOnTop: false, resizable: true },
        companion: { x: 1000, y: 72, width: 400, height: 790, radius: 14, glass: true, alwaysOnTop: true, resizable: false },
        pill: { x: 520, y: 800, width: 400, height: 52, radius: 26, glass: true, alwaysOnTop: true, resizable: false },
      };
      let current = { x: 40, y: 52, width: 1360, height: 828 };
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        invoke(command, args = {}) {
          if (command === "daemon_snapshot") {
            return Promise.resolve(snapshot);
          }
          if (command === "prepare_shell_form") {
            return Promise.resolve(targets[args.form]);
          }
          if (command === "shell_metrics") {
            return Promise.resolve({ ...current });
          }
          if (command === "set_shell_bounds") {
            current = { x: args.x, y: args.y, width: args.width, height: args.height };
            setTimeout(() => window.__resize(args.width, args.height), lag);
            return Promise.resolve(null);
          }
          return Promise.resolve(null);
        },
      };
    },
    { snapshot, lag },
  );
  await page.addInitScript(() => {
    window.__clip = 0;
    window.__shape = null;
    const sample = () => {
      const el = document.querySelector(".composer");
      if (el) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) {
          const over = Math.max(0, -r.left, -r.top, r.right - innerWidth, r.bottom - innerHeight);
          window.__clip = Math.max(window.__clip, over);
          // F1: a stub composer or a jump. The old pill->full pinned it at y=0
          // and narrowed it 400->20px for up to 17 frames, then jumped 746px.
          // y=0 is a stub only in a window taller than the composer: the pill
          // composer is the whole 400x52 window. Every form's composer is at
          // least 376px wide. The jump is per display frame (16.7ms; a frame
          // the headless resize stub stalls counts as the frames it spans), in
          // window terms: left/right insets, width and the gap under it.
          const s = window.__shape;
          if (s) {
            const now = performance.now();
            const cur = { l: r.left, r: innerWidth - r.right, b: innerHeight - r.bottom, w: r.width, y: r.top, t: now };
            if (s.prev) {
              const frames = Math.max(1, Math.round((now - s.prev.t) / (1000 / 60)));
              const jump = Math.max(...["l", "r", "b", "w"].map((k) => Math.abs(cur[k] - s.prev[k]))) / frames;
              if (jump > s.jump) {
                s.jump = jump;
                s.at = { frames, raw: Math.round(jump * frames) };
              }
            }
            s.minW = Math.min(s.minW, r.width);
            if (r.top < 1 && innerHeight > r.height + 24) {
              s.stubs += 1;
            }
            s.frames += 1;
            s.prev = cur;
          }
        }
      }
      requestAnimationFrame(sample);
    };
    // rAF only: resize events run before rAF in a frame, so this sees what
    // is painted after the app has re-laid the composer.
    requestAnimationFrame(sample);
  });
  await page.goto(origin, { waitUntil: "networkidle" });
  const settle = async () => {
    await page.waitForTimeout(200);
    for (let i = 0; i < 40 && Date.now() - last < 700; i += 1) {
      await page.waitForTimeout(100);
    }
    await page.waitForTimeout(900);
  };
  const measure = () =>
    page.evaluate(() => {
      const box = (selector) => {
        const r = document.querySelector(selector).getBoundingClientRect();
        return { x: r.x, y: r.y, w: r.width, h: r.height };
      };
      const composer = getComputedStyle(document.querySelector(".composer"));
      return {
        vp: { w: innerWidth, h: innerHeight },
        form: document.querySelector(".win").dataset.shellForm,
        win: box(".win"),
        composer: box(".composer"),
        bg: composer.backgroundColor,
        filter: composer.backdropFilter,
      };
    });
  const check = (m, form) => {
    if (m.form !== form) {
      throw new Error(`lag ${lag}: expected ${form}, got ${m.form}`);
    }
    if (form !== "pill" && (Math.abs(m.win.w - m.vp.w) > 1 || Math.abs(m.win.h - m.vp.h) > 1)) {
      throw new Error(`lag ${lag}: ${form} window is ${m.win.w}x${m.win.h} in a ${m.vp.w}x${m.vp.h} viewport`);
    }
    if (m.composer.x < -1 || m.composer.x + m.composer.w > m.vp.w + 1 || m.composer.y + m.composer.h > m.vp.h + 1) {
      throw new Error(`lag ${lag}: ${form} composer ${JSON.stringify(m.composer)} leaves the ${m.vp.w}x${m.vp.h} viewport`);
    }
    if (!/^rgb\(/.test(m.bg) || (m.filter && m.filter !== "none")) {
      throw new Error(`lag ${lag}: native composer is not the opaque fallback (${m.bg}, ${m.filter})`);
    }
  };
  const clips = {};
  const shape = {};
  const step = async (name, act, form) => {
    await page.evaluate(() => {
      window.__clip = 0;
      window.__shape = { prev: null, jump: 0, minW: Infinity, stubs: 0, frames: 0 };
    });
    await act();
    // The morph has ended when the viewport is the form's target size.
    const size = { full: [1360, 828], companion: [400, 790], pill: [400, 52] }[form];
    await page.waitForFunction(
      ([want, w, h]) => document.querySelector(".win").dataset.shellForm === want && innerWidth === w && innerHeight === h,
      [form, ...size],
      { timeout: 8000 },
    );
    await settle();
    check(await measure(), form);
    clips[name] = Math.round(await page.evaluate(() => window.__clip));
    const s = await page.evaluate(() => window.__shape);
    shape[name] = { jump: Math.round(s.jump), minW: Math.round(s.minW), stubs: s.stubs, frames: s.frames, at: s.at };
    if (s.stubs > 0 || s.minW < 300 || s.jump > MAX_JUMP) {
      throw new Error(`lag ${lag}: ${name} composer went degenerate or jumped ${JSON.stringify(shape[name])} (max jump ${MAX_JUMP}px/frame)`);
    }
  };
  check(await measure(), "full");
  await step("full>companion", () => page.locator('button[aria-label="Companion window"]').click(), "companion");
  await step("companion>pill", () => page.locator('button[aria-label="Float as a pill"]').click(), "pill");
  await step("pill>companion", () => page.keyboard.press("Escape"), "companion");
  await step("companion>full", () => page.keyboard.press("Escape"), "full");
  await step("full>pill", () => page.locator('button[aria-label="Float as a pill"]').click(), "pill");
  await step("pill>full", () => page.keyboard.press("Escape"), "full");
  console.log(`native-lag-smoke: lag ${lag} mid-morph composer clip px ${JSON.stringify(clips)}`);
  console.log(`native-lag-smoke: lag ${lag} composer per frame ${JSON.stringify(shape)}`);
  const worst = Object.entries(clips).filter(([, px]) => px > 1);
  if (worst.length) {
    throw new Error(`lag ${lag}: composer clipped mid-morph ${JSON.stringify(Object.fromEntries(worst))}`);
  }
  await context.close();
}

const browser = await launchBrowser();
try {
  await waitForHttp(origin);
  for (const lag of [0, 40, 120]) {
    await run(browser, lag);
  }
  console.log("native-lag-smoke: pass");
} finally {
  await browser.close();
  preview.kill();
}
