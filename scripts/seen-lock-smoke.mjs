/**
 * Seen-lock smoke. A blind click, a hold after the window blurs, and
 * visibility:hidden mid-hold must not POST a decision. Keyboard approval
 * only counts when the card itself is focused.
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

function drain(stream) {
  stream.on("data", () => {});
}

async function waitForHttp(url, child) {
  const deadline = Date.now() + 30_000;
  let exited = null;
  child.on("exit", (code) => {
    exited = code ?? 0;
  });
  while (Date.now() < deadline) {
    if (exited !== null) {
      throw new Error(`vite preview exited ${exited}`);
    }
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1000) });
      if (response.status < 500) {
        return;
      }
    } catch {
      // Preview is not accepting connections yet.
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`preview did not respond at ${url}`);
}

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
const preview = spawn(
  process.execPath,
  [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: desktop, stdio: ["ignore", "pipe", "pipe"] },
);
drain(preview.stdout);
drain(preview.stderr);

const now = Date.UTC(2026, 8, 30, 15, 14);
const fixture = {
  protocol: 1,
  role: "server",
  node: "seen-lock-smoke",
  provider: "mock",
  provider_detail: "mock",
  sync: "stub",
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
  approvals: [
    {
      id: "ap_seen",
      job_id: "job_seen",
      agent_id: "reviewer",
      agent_name: "Reviewer",
      thread_id: "thread_reviewer",
      effect_class: "external",
      action: "post_pr_comment",
      purpose: "Leave a note on the pull request.",
      draft: "Please refresh() the branch before review.",
      evidence: { repo: "DasVR/NIL", ref: "212", event_id: "ev_abcdef", kind: "repo.push" },
      evidence_text: "repo DasVR/NIL",
      status: "pending",
      provider: "mock",
      model: "mock-review-v0",
      usage_kind: "estimated",
      input_tokens: 1200,
      output_tokens: 80,
      micro_usd: 0,
      created_at: now,
      expires_at: Date.now() + 15 * 60 * 1000,
      decided_at: null,
      decision_event_id: null,
      reason: null,
      committed: false,
      undo_until: null,
    },
  ],
  ledger: [],
  events: [
    {
      id: "ev_abcdef",
      version: 1,
      hlc: `${now}:0:node`,
      source: "demo",
      kind: "approval.requested",
      thread_id: "thread_reviewer",
      idempotency_key: "approval-requested:ap_seen",
    },
  ],
};

let decisions = 0;
let browser;

function fail(message) {
  throw new Error(message);
}

try {
  await waitForHttp(origin, preview);
  browser = await chromium.launch({ channel: "chrome", headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const logs = [];
  page.on("pageerror", (error) => logs.push(String(error)));

  await page.addInitScript(() => {
    const tick = () => {
      const card = document.querySelector("article.card");
      const button = card?.querySelector("button.approve");
      if (!card || !button || window.__blind) {
        requestAnimationFrame(tick);
        return;
      }
      window.__blind = {
        seen: card.getAttribute("data-seen"),
        hello: card.getAttribute("data-hello"),
      };
      button.click();
    };
    requestAnimationFrame(tick);
  });

  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(fixture),
      });
      return;
    }
    if (url.pathname.endsWith("/decision") && route.request().method() === "POST") {
      decisions += 1;
      await route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
      return;
    }
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });

  await page.goto(origin, { waitUntil: "networkidle" });
  await page.locator("article.card button.approve").waitFor();
  await page.waitForFunction(() => window.__blind);
  const blind = await page.evaluate(() => window.__blind);
  if (blind.seen === "armed") {
    fail(`blind click landed after the seen lock (${blind.seen})`);
  }
  await page.waitForTimeout(400);
  if (decisions !== 0) {
    fail(`blind click posted ${decisions} decision(s)`);
  }

  const card = page.locator("article.card");
  await card.focus();
  try {
    await page.waitForFunction(
      () => document.querySelector("article.card")?.getAttribute("data-seen") === "armed",
      null,
      { timeout: 4000 },
    );
  } catch (error) {
    const diag = await page.evaluate(async () => {
      const sample = () => {
        const node = document.querySelector("article.card");
        const evidence = node?.querySelector(".evidence");
        const style = node ? getComputedStyle(node) : null;
        return {
          now: performance.now(),
          seen: node?.getAttribute("data-seen"),
          since: node?.getAttribute("data-seen-since"),
          focus: document.hasFocus(),
          hidden: document.hidden,
          opacity: style?.opacity ?? null,
          visibility: style?.visibility ?? null,
        };
      };
      const first = sample();
      await new Promise((resolve) => setTimeout(resolve, 1000));
      return { first, second: sample(), log: window.__seen ?? [] };
    });
    fail(`${error.message} ${JSON.stringify(diag)}`);
  }

  const link = page.locator(".review-link");
  await link.focus();
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(700);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  if (decisions !== 0) {
    fail(`global shortcut posted ${decisions} decision(s)`);
  }
  const helloAfterGlobal = await card.getAttribute("data-hello");
  if (helloAfterGlobal === "open") {
    fail("global shortcut opened Windows Hello");
  }

  await card.focus();
  await page.waitForFunction(
    () => document.querySelector("article.card")?.getAttribute("data-seen") === "armed",
    null,
    { timeout: 4000 },
  );
  const before = await page.locator("article.card button.approve").boundingBox();
  if (!before) {
    fail("approve button had no box");
  }
  await page.mouse.move(before.x + before.width / 2, before.y + before.height / 2);
  await page.mouse.down();
  const after = await page.locator("article.card button.approve").boundingBox();
  await page.mouse.up();
  if (!after) {
    fail("approve button disappeared on mousedown");
  }
  const shift = Math.abs(after.y - before.y);
  if (shift > 1) {
    fail(`approve button shifted ${shift}px when the focus hint updated`);
  }
  await page.waitForTimeout(500);
  if (decisions !== 0) {
    fail(`mousedown/up posted ${decisions} decision(s) before Hello confirm`);
  }

  const helloOpen = await card.getAttribute("data-hello");
  if (helloOpen === "open") {
    await page.locator(".hello-actions button", { hasText: "Cancel" }).click();
    await page.waitForFunction(
      () => document.querySelector("article.card")?.getAttribute("data-hello") !== "open",
    );
  }

  await card.focus();
  await page.waitForFunction(
    () => document.querySelector("article.card")?.getAttribute("data-seen") === "armed",
    null,
    { timeout: 4000 },
  );
  await page.evaluate(() => {
    window.dispatchEvent(new Event("blur"));
  });
  const seenAfterBlur = await card.getAttribute("data-seen");
  if (seenAfterBlur === "armed") {
    fail("blur left the seen lock armed");
  }
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(700);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  if (decisions !== 0) {
    fail(`hold after blur posted ${decisions} decision(s)`);
  }
  if ((await card.getAttribute("data-hello")) === "open") {
    fail("hold after blur opened Windows Hello");
  }

  await page.evaluate(() => {
    window.dispatchEvent(new Event("focus"));
  });
  await card.focus();
  await page.waitForFunction(
    () => document.querySelector("article.card")?.getAttribute("data-seen") === "armed",
    null,
    { timeout: 4000 },
  );
  await page.evaluate(() => {
    const node = document.querySelector("article.card");
    node.focus();
    node.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }),
    );
    node.style.visibility = "hidden";
  });
  await page.waitForTimeout(800);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  if (decisions !== 0) {
    fail(`visibility hidden mid-hold posted ${decisions} decision(s)`);
  }
  const helloHidden = await page.evaluate(() => document.querySelector("article.card")?.getAttribute("data-hello"));
  if (helloHidden === "open") {
    fail("visibility hidden mid-hold opened Windows Hello");
  }
  const seenHidden = await page.evaluate(() => document.querySelector("article.card")?.getAttribute("data-seen"));
  if (seenHidden === "armed") {
    fail("visibility hidden left the seen lock armed");
  }

  if (logs.length > 0) {
    fail(logs.join("\n"));
  }
  console.log("seen-lock-smoke: pass");
  console.log(`decisions posted: ${decisions}`);
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
