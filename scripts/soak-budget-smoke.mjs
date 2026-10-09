/**
 * Long-run budget. A demo that has filed hundreds of cards must cost the same
 * as a fresh one:
 * - the stream renders at most 40 rows, so the DOM stays bounded;
 * - filed receipts carry no backdrop-filter and no running interval;
 * - one snapshot request is in flight at a time, even when the daemon is slow;
 * - a hidden window does not poll, and polls at once when shown;
 * - the hidden settings and voice windows mount nothing.
 * The Tauri IPC is a stub that serves a large snapshot.
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

const now = Date.UTC(2026, 9, 2, 22, 0, 0);
const approvals = [];
const events = [];
for (let n = 0; n < 300; n += 1) {
  const id = `ap_${n}`;
  approvals.push({
    id,
    job_id: `job_${n}`,
    agent_id: "reviewer",
    agent_name: "Reviewer",
    thread_id: `th_${n}`,
    effect_class: "external",
    action: "post_pr_comment",
    purpose: "External effect on DasVR/NIL is denied until Windows Hello verifies the signer.",
    draft: "Looks good.",
    evidence: { repo: "DasVR/NIL", ref: "phase0", event_id: `ev_${n}`, kind: "repo.push" },
    evidence_text: "repo DasVR/NIL",
    status: "denied",
    provider: "mock",
    model: "mock-review-v0",
    usage_kind: "estimated",
    input_tokens: 0,
    output_tokens: 0,
    micro_usd: 0,
    created_at: now - n * 1000,
    expires_at: null,
    decided_at: now - n * 1000,
    decision_event_id: null,
    reason: "external is denied until Windows Hello is the production verifier",
    committed: true,
    undo_until: null,
  });
  events.push({
    id: `ev_req_${n}`,
    version: 1,
    hlc: `${now - n * 1000}:0:node`,
    source: "runtime",
    kind: "approval.requested",
    thread_id: `th_${n}`,
    idempotency_key: `approval-requested:${id}`,
  });
}
const snapshot = {
  protocol: 1,
  role: "executor",
  node: "budget-smoke",
  provider: "mock",
  provider_detail: "mock provider (demo, no network)",
  sync: "stub",
  endpoint_id: null,
  agents: [{ id: "reviewer", name: "Reviewer", project: "DasVR/NIL", persona: "", token_cap: 20000, tokens_spent: 0, status: "idle" }],
  approvals,
  ledger: [],
  events,
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

function stub(label, snapshot, slowMs) {
  return [
    ({ label, snapshot, slowMs }) => {
      window.__polls = { started: 0, inFlight: 0, maxInFlight: 0 };
      const realSetInterval = window.setInterval.bind(window);
      const realClearInterval = window.clearInterval.bind(window);
      window.__intervals = new Set();
      window.setInterval = (fn, ms, ...rest) => {
        const id = realSetInterval(fn, ms, ...rest);
        window.__intervals.add(id);
        return id;
      };
      window.clearInterval = (id) => {
        window.__intervals.delete(id);
        realClearInterval(id);
      };
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label } },
        invoke(command) {
          if (command === "daemon_snapshot") {
            window.__polls.started += 1;
            window.__polls.inFlight += 1;
            window.__polls.maxInFlight = Math.max(window.__polls.maxInFlight, window.__polls.inFlight);
            return new Promise((resolve) =>
              setTimeout(() => {
                window.__polls.inFlight -= 1;
                resolve(snapshot);
              }, slowMs),
            );
          }
          if (command === "prepare_shell_form") {
            return Promise.resolve({ x: 0, y: 0, width: 1360, height: 828, radius: 14, glass: false, alwaysOnTop: false, resizable: true });
          }
          if (command === "shell_metrics") {
            return Promise.resolve({ x: 0, y: 0, width: 1360, height: 828 });
          }
          return Promise.resolve(null);
        },
      };
    },
    { label, snapshot, slowMs },
  ];
}

const browser = await launchBrowser();
try {
  await waitForHttp(origin);
  const context = await browser.newContext({ viewport: { width: 1360, height: 828 } });
  const page = await context.newPage();
  await page.addInitScript(...stub("main", snapshot, 2500));
  await page.goto(origin, { waitUntil: "load" });
  await page.waitForFunction(() => document.querySelectorAll("article.card").length > 0, null, { timeout: 15000 });
  await page.waitForTimeout(6000);
  const main = await page.evaluate(() => {
    let backdrop = 0;
    for (const el of document.querySelectorAll("article.card")) {
      const value = getComputedStyle(el).backdropFilter;
      if (value && value !== "none") {
        backdrop += 1;
      }
    }
    return {
      rows: document.querySelectorAll(".stream > li").length,
      cards: document.querySelectorAll("article.card").length,
      nodes: document.getElementsByTagName("*").length,
      backdrop,
      intervals: window.__intervals.size,
      polls: { ...window.__polls },
    };
  });
  if (main.rows > 40) {
    throw new Error(`the stream rendered ${main.rows} rows for 300 events`);
  }
  if (main.nodes > 2500) {
    throw new Error(`the main window has ${main.nodes} elements for 300 filed cards`);
  }
  if (main.backdrop !== 0) {
    throw new Error(`${main.backdrop} filed receipts carry a backdrop-filter`);
  }
  if (main.intervals > 3) {
    throw new Error(`${main.intervals} intervals are running with no undo window open`);
  }
  if (main.polls.maxInFlight !== 1 || main.polls.started > 4) {
    throw new Error(`snapshot polls overlapped against a slow daemon ${JSON.stringify(main.polls)}`);
  }

  // A hidden window stops polling and refreshes as soon as it is shown.
  const hidden = await context.newPage();
  await hidden.addInitScript(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => window.__vis ?? "visible" });
  });
  await hidden.addInitScript(...stub("card", { ...snapshot, approvals: [], events: [] }, 0));
  await hidden.goto(origin, { waitUntil: "load" });
  await hidden.waitForTimeout(1500);
  await hidden.evaluate(() => {
    window.__vis = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await hidden.waitForTimeout(1200);
  const atHide = await hidden.evaluate(() => window.__polls.started);
  await hidden.waitForTimeout(3500);
  const whileHidden = await hidden.evaluate(() => window.__polls.started);
  if (whileHidden !== atHide) {
    throw new Error(`a hidden window polled ${whileHidden - atHide} times`);
  }
  await hidden.evaluate(() => {
    window.__vis = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await hidden.waitForTimeout(200);
  if ((await hidden.evaluate(() => window.__polls.started)) <= whileHidden) {
    throw new Error("a window shown again did not refresh at once");
  }

  for (const label of ["settings", "voice"]) {
    const idle = await context.newPage();
    await idle.addInitScript(...stub(label, snapshot, 0));
    await idle.goto(origin, { waitUntil: "load" });
    await idle.waitForTimeout(2000);
    const state = await idle.evaluate(() => ({ children: document.getElementById("app")?.children.length ?? -1, polls: window.__polls.started }));
    if (state.children !== 0 || state.polls !== 0) {
      throw new Error(`the hidden ${label} window mounted the app ${JSON.stringify(state)}`);
    }
  }
  console.log(`soak-budget-smoke: pass ${JSON.stringify(main)}`);
} finally {
  await browser.close();
  preview.kill();
}
