/**
 * Only the card window decides.
 * - Tauri label "card": the card window renders the waiting card, and a hold
 *   after the seen lock calls sign_decision once.
 * - Tauri label "main": the same hold only calls open_card_window. The main
 *   window never calls sign_decision or undo_decision.
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

async function holdApprove(page, label) {
  await page.addInitScript(
    ({ label, snapshot }) => {
      window.__invokes = [];
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label } },
        invoke(command, args = {}) {
          window.__invokes.push({ command, args });
          if (command === "daemon_snapshot") {
            return Promise.resolve(snapshot);
          }
          if (command === "session_token") {
            return Promise.resolve("");
          }
          return Promise.resolve(null);
        },
      };
    },
    { label, snapshot },
  );
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Approve draft" }).waitFor();
  await page.evaluate(() => {
    document.querySelector("article.card button.approve")?.click();
  });
  await page.keyboard.press("Alt+ArrowDown");
  await page.waitForFunction(() => Boolean(document.querySelector("article.card .hold-hint.armed")), null, {
    timeout: 6000,
  });
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(900);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  await page.waitForTimeout(400);
  return page.evaluate(() => window.__invokes.map((entry) => entry.command).filter((c) => c !== "daemon_snapshot"));
}

const browser = await launchBrowser();
try {
  await waitForHttp(origin);
  const cardPage = await browser.newPage({ viewport: { width: 560, height: 760 } });
  const fromCard = await holdApprove(cardPage, "card");
  if (fromCard.filter((c) => c === "sign_decision").length !== 1 || fromCard.includes("open_card_window")) {
    throw new Error(`card window did not sign once: ${JSON.stringify(fromCard)}`);
  }
  const mainPage = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const fromMain = await holdApprove(mainPage, "main");
  if (fromMain.includes("sign_decision") || fromMain.includes("undo_decision")) {
    throw new Error(`main window decided: ${JSON.stringify(fromMain)}`);
  }
  if (!fromMain.includes("open_card_window")) {
    throw new Error(`main window did not open the card window: ${JSON.stringify(fromMain)}`);
  }
  console.log("card-window-smoke: pass");
} finally {
  await browser.close();
  preview.kill();
}
