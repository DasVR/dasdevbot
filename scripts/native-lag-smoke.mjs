/**
 * Native resize lag. A Tauri window applies set_shell_bounds a little later
 * than the webview asks for it. With 0, 40 and 120ms of lag the layout must
 * settle on the native window's size (no stale 1360x828 inside a 400x790
 * companion), and the native composer must use the opaque fallback fill,
 * since a transparent window with no acrylic gives the blur nothing to sample.
 * The Tauri IPC is a stub; set_shell_bounds resizes the page after `lag` ms.
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
          if (command === "session_token") {
            return Promise.resolve("");
          }
          return Promise.resolve(null);
        },
      };
    },
    { snapshot, lag },
  );
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
  check(await measure(), "full");
  await page.locator('button[aria-label="Companion window"]').click();
  await settle();
  check(await measure(), "companion");
  await page.locator('button[aria-label="Float as a pill"]').click();
  await settle();
  check(await measure(), "pill");
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
