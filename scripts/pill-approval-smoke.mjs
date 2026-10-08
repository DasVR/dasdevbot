/**
 * The main window never decides (UX 2) and its shell keyboard is sound.
 * - Full, companion and pill: no Approve control and no focusable decision
 *   card. A waiting card is a thread step; nothing calls sign_decision or
 *   undo_decision.
 * - Pill "1 waiting" morphs to full and focuses the waiting step, never body,
 *   and asks the shell for the card window (UX 1).
 * - Escape: companion -> full, pill -> the form before it, a thread item ->
 *   the stream (UX 3).
 * - Tab never lands in a hidden layer, the pill's window, or the collapsed
 *   roster (UX 5).
 * The deciding card itself is covered by card-window-smoke.
 * The Tauri IPC is a stub (label "main"); set_shell_bounds resizes the page.
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

const now = Date.UTC(2026, 8, 30, 13, 4, 0);
const card = {
  id: "ap_pill",
  job_id: "job_pill",
  agent_id: "reviewer",
  agent_name: "Reviewer",
  thread_id: "thread_pill",
  effect_class: "external",
  action: "post_pr_comment",
  purpose: "Leave a note on the pull request.",
  draft: "Hello",
  evidence: { repo: "DasVR/NIL", ref: "phase0", event_id: "ev_abcdef", kind: "repo.push" },
  evidence_text: "repo DasVR/NIL",
  status: "pending",
  provider: "mock",
  model: "mock-review-v0",
  usage_kind: "estimated",
  input_tokens: 1200,
  output_tokens: 80,
  micro_usd: 0,
  created_at: now,
  expires_at: null,
  decided_at: null,
  decision_event_id: null,
  reason: null,
  committed: false,
  undo_until: null,
};


const snapshot = {
  protocol: 1,
  role: "server",
  node: "pill-smoke",
  provider: "mock",
  provider_detail: "mock",
  sync: "stub",
  endpoint_id: null,
  agents: [
    {
      id: "reviewer",
      name: "Reviewer",
      project: "DasVR/NIL",
      persona: "Reviews the branch before anything is posted.",
      token_cap: 20000,
      tokens_spent: 1280,
      status: "blocked",
    },
  ],
  approvals: [card],
  ledger: [],
  events: [
    {
      id: "ev_ask",
      version: 1,
      hlc: "1759241041000:0:node",
      source: "demo",
      kind: "approval.requested",
      thread_id: "thread_pill",
      idempotency_key: `approval-requested:${card.id}`,
    },
  ],
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

const browser = await launchBrowser();
try {
  await waitForHttp(origin);
  const context = await browser.newContext({ viewport: { width: 1360, height: 828 } });
  const page = await context.newPage();
  await page.exposeFunction("__resize", async (w, h) => {
    await page.setViewportSize({ width: Math.max(1, Math.round(w)), height: Math.max(1, Math.round(h)) });
  });
  await page.addInitScript((snapshot) => {
    window.__invokes = [];
    const targets = {
      full: { x: 40, y: 52, width: 1360, height: 828, radius: 14, glass: false, alwaysOnTop: false, resizable: true },
      companion: { x: 1000, y: 72, width: 400, height: 790, radius: 14, glass: true, alwaysOnTop: true, resizable: false },
      pill: { x: 520, y: 800, width: 400, height: 52, radius: 26, glass: true, alwaysOnTop: true, resizable: false },
    };
    let current = { x: 40, y: 52, width: 1360, height: 828 };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" } },
      invoke(command, args = {}) {
        window.__invokes.push(command);
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
          window.__resize(args.width, args.height);
          return Promise.resolve(null);
        }
        return Promise.resolve(null);
      },
    };
  }, snapshot);
  await page.goto(origin, { waitUntil: "load" });
  await page.locator("[data-waiting='ap_pill']").waitFor();

  const form = () => page.evaluate(() => document.querySelector(".win")?.dataset.shellForm ?? "");
  const settle = async (want) => {
    await page.waitForFunction(
      (want) =>
        document.querySelector(".win")?.dataset.shellForm === want &&
        document.getAnimations().every((a) => a.playState !== "running"),
      want,
      { timeout: 8000 },
    );
    await page.waitForTimeout(150);
  };
  const click = async (label) => {
    await page.locator(`button[aria-label="${label}"]`).first().click();
  };
  async function noDecisionControl(where) {
    const state = await page.evaluate(() => ({
      approve: document.querySelectorAll("button.approve, button.deny").length,
      focusableCard: document.querySelectorAll("article.card[tabindex]").length,
    }));
    if (state.approve !== 0 || state.focusableCard !== 0) {
      throw new Error(`${where}: the main window drew a decision control ${JSON.stringify(state)}`);
    }
  }
  async function tabWalk(where) {
    const bad = [];
    await page.evaluate(() => {
      if (document.activeElement instanceof HTMLElement) {
        document.activeElement.blur();
      }
    });
    for (let i = 0; i < 40; i += 1) {
      await page.keyboard.press("Tab");
      const hit = await page.evaluate(() => {
        const el = document.activeElement;
        if (!(el instanceof HTMLElement) || el === document.body) {
          return null;
        }
        const form = document.querySelector(".win")?.dataset.shellForm;
        const hidden = !el.checkVisibility({ opacityProperty: true, visibilityProperty: true });
        const inRoster = Boolean(el.closest(".roster"));
        const inWin = Boolean(el.closest(".win"));
        return {
          what: `${el.tagName.toLowerCase()}.${el.className}`.slice(0, 60),
          inert: Boolean(el.closest("[inert]")),
          hidden,
          roster: inRoster && form !== "full",
          pillWin: inWin && form === "pill",
        };
      });
      if (hit && (hit.inert || hit.hidden || hit.roster || hit.pillWin)) {
        bad.push(hit);
      }
    }
    if (bad.length) {
      throw new Error(`${where}: Tab reached hidden or inert controls ${JSON.stringify(bad.slice(0, 3))}`);
    }
  }

  // Full form.
  await settle("full");
  await noDecisionControl("full");
  await tabWalk("full");

  // Companion, then Escape back to full.
  await click("Companion window");
  await settle("companion");
  await noDecisionControl("companion");
  await tabWalk("companion");
  await page.keyboard.press("Escape");
  await settle("full");

  // Pill from full; Escape returns to full.
  await click("Float as a pill");
  await settle("pill");
  await noDecisionControl("pill");
  await tabWalk("pill");
  await page.keyboard.press("Escape");
  await settle("full");

  // Pill from companion; Escape returns to companion.
  await click("Companion window");
  await settle("companion");
  await click("Float as a pill");
  await settle("pill");
  await page.keyboard.press("Escape");
  await settle("companion");
  await page.keyboard.press("Escape");
  await settle("full");

  // Pill "1 waiting" opens full on the waiting step and asks for the card window.
  await click("Float as a pill");
  await settle("pill");
  const wt = page.locator(".wt");
  if (!(await wt.isVisible()) || !/1 waiting/.test((await wt.textContent()) ?? "")) {
    throw new Error(`pill does not show 1 waiting: ${await wt.textContent()}`);
  }
  await wt.click();
  await settle("full");
  const focus = await page.evaluate(() => ({
    body: document.activeElement === document.body,
    waiting: document.activeElement?.matches("[data-waiting='ap_pill']") ?? false,
  }));
  if (focus.body || !focus.waiting) {
    throw new Error(`1 waiting did not focus the waiting step ${JSON.stringify(focus)}`);
  }
  await page.waitForFunction(() => window.__invokes.includes("open_card_window"), null, { timeout: 3000 });

  // Escape on a thread item moves focus to the stream.
  await page.keyboard.press("Escape");
  if (!(await page.evaluate(() => document.activeElement?.matches(".stream-slot") ?? false))) {
    throw new Error("Escape on a thread item did not move focus to the stream");
  }
  if ((await form()) !== "full") {
    throw new Error("Escape in full changed the form");
  }

  const invokes = await page.evaluate(() => window.__invokes);
  if (invokes.includes("sign_decision") || invokes.includes("undo_decision")) {
    throw new Error(`the main window decided ${JSON.stringify(invokes.filter((c) => c.endsWith("decision")))}`);
  }
  console.log("pill-approval-smoke: pass");
} finally {
  await browser.close();
  preview.kill();
}
