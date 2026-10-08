/**
 * One decision point: the card window.
 * Card window (Tauri label "card"):
 * - a blind click or Enter before the seen lock does nothing;
 * - the hint row is reserved, so the buttons do not move when it appears (UX 6);
 * - the native sight signal disarms the seen lock and a fresh dwell re-arms it;
 * - Enter on the focused, armed Approve approves (UX 7);
 * - while sign_decision is in flight the card shows a plain
 *   "Confirm with Windows Hello" line and no button (C4, UX 10);
 * - a cancelled Hello prompt returns to the waiting card with no error;
 * - Escape: deny reason -> Back with focus on the card, card -> the window (UX 3);
 * - a destructive approval is the flat ink row only (C1, UX 8).
 * Main window (label "main"): no Approve control exists in any form, the
 * waiting step only calls open_card_window, and nothing calls sign_decision
 * or undo_decision (UX 2).
 * The Tauri IPC is a stub that records every invoke.
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
  id: "ap_card",
  job_id: "job_pill",
  agent_id: "reviewer",
  agent_name: "Reviewer",
  thread_id: "thread_pill",
  effect_class: "write_local",
  action: "edit",
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

const destructive = {
  ...card,
  id: "ap_force",
  effect_class: "destructive",
  action: "force_push",
  purpose: "Force-push phase0. This rewrites the remote branch.",
  draft: "git push --force origin phase0",
};

const snapshot = {
  protocol: 1,
  role: "executor",
  node: "card-smoke",
  provider: "mock",
  provider_detail: "mock provider (demo, no network)",
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

function stub({ label, snapshot, helloMs = 0, cancel = false }) {
  return [
    ({ label, snapshot, helloMs, cancel }) => {
      window.__invokes = [];
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label } },
        invoke(command, args = {}) {
          window.__invokes.push({ command, args });
          if (command === "daemon_snapshot") {
            return Promise.resolve(snapshot);
          }
          if (command === "prepare_shell_form") {
            return Promise.resolve({ x: 0, y: 0, width: 1280, height: 800, radius: 14, glass: false, alwaysOnTop: false, resizable: true });
          }
          if (command === "shell_metrics") {
            return Promise.resolve({ x: 0, y: 0, width: 1280, height: 800 });
          }
          if (command === "sign_decision") {
            return new Promise((resolve, reject) =>
              setTimeout(() => (cancel ? reject(new Error("Windows Hello consent was denied")) : resolve(null)), helloMs),
            );
          }
          return Promise.resolve(null);
        },
      };
    },
    { label, snapshot, helloMs, cancel },
  ];
}

function signs(page) {
  return page.evaluate(() => window.__invokes.filter((entry) => entry.command === "sign_decision").length);
}

async function holdChord(page) {
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(900);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
}

async function openCard(browser, options) {
  const page = await browser.newPage({ viewport: { width: 560, height: 760 } });
  await page.addInitScript(...stub({ label: "card", ...options }));
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Approve draft" }).waitFor();
  return page;
}

async function arm(page) {
  await page.locator("article.card").focus();
  await page.waitForFunction(() => Boolean(document.querySelector("article.card .hold-hint.armed")), null, {
    timeout: 6000,
  });
}

const browser = await launchBrowser();
try {
  await waitForHttp(origin);

  // Blind input, reserved hint row, native sight, then Hello in flight.
  const page = await openCard(browser, { snapshot, helloMs: 700 });
  const before = await page.locator("article.card .actions").boundingBox();
  await page.locator("article.card button.approve").click();
  await page.locator("article.card button.approve").press("Enter");
  if ((await signs(page)) !== 0) {
    throw new Error("a click or Enter before the seen lock signed");
  }
  await arm(page);
  const after = await page.locator("article.card .actions").boundingBox();
  if (!before || !after || Math.abs(before.y - after.y) > 0.5) {
    throw new Error(`the buttons moved when the hint appeared ${before?.y} -> ${after?.y}`);
  }
  await page.evaluate(() =>
    window.dispatchEvent(new CustomEvent("dasdevbot:native-sight", { detail: { visible: false } })),
  );
  await page.waitForFunction(() => !document.querySelector("article.card .hold-hint.armed"), null, { timeout: 2000 });
  await page.locator("article.card").focus();
  await holdChord(page);
  if ((await signs(page)) !== 0) {
    throw new Error("a hold signed while the native window was not visible");
  }
  const t0 = Date.now();
  await page.evaluate(() =>
    window.dispatchEvent(new CustomEvent("dasdevbot:native-sight", { detail: { visible: true } })),
  );
  await arm(page);
  if (Date.now() - t0 < 700) {
    throw new Error("seen lock re-armed without a fresh dwell");
  }
  await page.locator("article.card button.approve").focus();
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.querySelector("article.card .quiet .hello")?.textContent === "Confirm with Windows Hello", null, {
    timeout: 2000,
  });
  const hello = await page.evaluate(() => ({
    buttons: document.querySelectorAll("article.card .quiet button").length,
    hint: Boolean(document.querySelector("article.card .hold-hint")),
  }));
  if (hello.buttons !== 0 || hello.hint) {
    throw new Error(`Hello pending drew controls ${JSON.stringify(hello)}`);
  }
  if ((await signs(page)) !== 1) {
    throw new Error("Enter on the armed Approve did not sign exactly once");
  }
  await page.waitForFunction(() => !document.querySelector("article.card .quiet .hello"), null, { timeout: 3000 });
  await page.close();

  // Hello cancelled: back to waiting, no error.
  const cancelled = await openCard(browser, { snapshot, helloMs: 200, cancel: true });
  await arm(cancelled);
  await holdChord(cancelled);
  await cancelled.waitForFunction(() => !document.querySelector("article.card .quiet .hello"), null, { timeout: 3000 });
  const back = await cancelled.evaluate(() => ({
    approve: Boolean(document.querySelector("article.card button.approve")),
    note: document.querySelector(".card-window .note")?.textContent ?? "",
  }));
  if (!back.approve || back.note) {
    throw new Error(`a cancelled Hello did not return quietly to waiting ${JSON.stringify(back)}`);
  }

  // Escape: deny reason -> Back on the card, card -> window.
  await cancelled.locator("article.card button.deny").click();
  await cancelled.locator("article.card .reason input").waitFor();
  await cancelled.keyboard.press("Escape");
  const escaped = await cancelled.evaluate(() => ({
    reason: Boolean(document.querySelector("article.card .reason")),
    focus: document.activeElement?.matches("article.card") ?? false,
  }));
  if (escaped.reason || !escaped.focus) {
    throw new Error(`Escape did not go Back to the card ${JSON.stringify(escaped)}`);
  }
  await cancelled.keyboard.press("Escape");
  if (!(await cancelled.evaluate(() => document.activeElement?.matches("main.card-window") ?? false))) {
    throw new Error("Escape on the card did not move focus to the window");
  }
  await cancelled.close();

  // C1: destructive renders only as the flat row, in both windows.
  for (const label of ["card", "main"]) {
    const flat = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await flat.addInitScript(...stub({ label, snapshot: { ...snapshot, approvals: [destructive] } }));
    await flat.goto(origin, { waitUntil: "networkidle" });
    await flat.locator("article.card.flat").first().waitFor();
    const row = await flat.evaluate(() => {
      const card = document.querySelector("article.card.flat");
      return {
        text: card?.textContent ?? "",
        buttons: card?.querySelectorAll("button").length ?? -1,
        tab: card?.getAttribute("tabindex"),
        approve: document.querySelectorAll("button.approve, [data-waiting]").length,
      };
    });
    // CD ruling 3: "<Agent> wanted to force-push <ref>. Destructive actions are off in this build." + the mono command.
    if (
      !/\b\w+ wanted to force-push phase0\. Destructive actions are off in this build\./.test(row.text) ||
      !row.text.includes("git push --force origin phase0")
    ) {
      throw new Error(`${label}: destructive row copy ${JSON.stringify(row)}`);
    }
    if (row.buttons !== 0 || row.tab != null || row.approve !== 0) {
      throw new Error(`${label}: destructive row has controls ${JSON.stringify(row)}`);
    }
    await flat.close();
  }

  // Main window: no decision control in any form.
  const main = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await main.addInitScript(...stub({ label: "main", snapshot }));
  await main.goto(origin, { waitUntil: "networkidle" });
  await main.locator("[data-waiting='ap_card']").waitFor();
  if ((await main.locator("button.approve, article.card[tabindex]").count()) !== 0) {
    throw new Error("the main window drew a decision control");
  }
  await main.locator("[data-waiting='ap_card']").focus();
  await holdChord(main);
  await main.keyboard.press("Enter");
  await main.waitForTimeout(400);
  const fromMain = await main.evaluate(() => window.__invokes.map((entry) => entry.command));
  if (fromMain.includes("sign_decision") || fromMain.includes("undo_decision")) {
    throw new Error(`main window decided: ${JSON.stringify(fromMain)}`);
  }
  if (!fromMain.includes("open_card_window")) {
    throw new Error(`main window did not open the card window: ${JSON.stringify(fromMain)}`);
  }
  if (!(await main.evaluate(() => document.activeElement?.matches("[data-waiting='ap_card']") ?? false))) {
    throw new Error("focus left the waiting step after opening the card window");
  }
  console.log("card-window-smoke: pass");
} finally {
  await browser.close();
  preview.kill();
}
