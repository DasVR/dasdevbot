/**
 * A card that is not really visible must not approve.
 * Pill and companion: a blind click, Alt+Down plus a hold, and a mid-hold
 * sight loss all leave /v1/approvals/:id/decision uncalled.
 * The pill opens the card only through "1 waiting"; the hold then approves
 * Hello, and undo posts /undo.
 *
 * Decisions are Tauri IPC only. The page gets a stub `__TAURI_INTERNALS__`
 * whose `invoke` records `sign_decision` / `undo_decision` by forwarding them
 * to the routed stub paths below, so "posted /decision" here means "called
 * sign_decision". The daemon itself still answers HTTP decisions with 403.
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
const decisions = [];
const undos = [];

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

let liveApproval = { ...card };

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
  approvals: [liveApproval],
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

const preview = spawn("node", [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
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

function assertQuiet(label) {
  if (decisions.length !== 0) {
    throw new Error(`${label} sent ${JSON.stringify(decisions)}`);
  }
}

const browser = await launchBrowser();
try {
  await waitForHttp(origin);
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
      await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(snapshot) });
      return;
    }
    if (url.pathname.endsWith("/decision") && route.request().method() === "POST") {
      const body = await route.request().postDataJSON();
      decisions.push({ path: url.pathname, body });
      const nowMs = Date.now();
      liveApproval = {
        ...liveApproval,
        status: body.decision === "deny" ? "denied" : "approved",
        decided_at: nowMs,
        decision_event_id: "ev_a1b2c3",
        undo_until: nowMs + 60_000,
        committed: false,
        reason: body.reason ?? null,
      };
      snapshot.approvals = [liveApproval];
      await route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
      return;
    }
    if (url.pathname.endsWith("/undo") && route.request().method() === "POST") {
      undos.push({ path: url.pathname });
      liveApproval = { ...card };
      snapshot.approvals = [liveApproval];
      await route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
      return;
    }
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });
  await page.addInitScript(() => {
    const post = (path, body) =>
      fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }).then(
        () => null,
      );
    window.__TAURI_INTERNALS__ = {
      invoke(command, args = {}) {
        if (command === "daemon_snapshot") {
          return fetch("/v1/snapshot").then((response) => response.json());
        }
        if (command === "sign_decision") {
          return post(`/v1/approvals/${args.id}/decision`, { decision: args.decision, reason: args.reason });
        }
        if (command === "undo_decision") {
          return post(`/v1/approvals/${args.id}/undo`, {});
        }
        return Promise.reject(new Error(`unexpected invoke ${command}`));
      },
    };
  });
  await page.goto(`${origin}/?shellStage=1`, { waitUntil: "networkidle" });
  await page.locator(".wordmark").waitFor();
  await page.getByRole("button", { name: "Approve draft" }).waitFor();

  async function approvePoint() {
    await page.evaluate(() => window.__shellStage.snap("full"));
    const point = await page.evaluate(() => {
      const button = document.querySelector("article.card button.approve");
      if (!button) {
        return null;
      }
      const rect = button.getBoundingClientRect();
      return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    });
    if (!point) {
      throw new Error("Approve button is missing in the full form");
    }
    return point;
  }

  async function collapsed(form) {
    const state = await page.evaluate((next) => {
      const win = document.querySelector(".win");
      const article = document.querySelector("article.card");
      const style = article ? getComputedStyle(article) : null;
      const stream = document.querySelector(".stream-slot");
      return {
        form: win instanceof HTMLElement ? win.dataset.shellForm ?? "" : "",
        cardVisibility: style ? style.visibility : "",
        armed: Boolean(article?.querySelector(".hold-hint.armed")),
        focused: document.activeElement === article,
        streamInert: stream instanceof HTMLElement ? stream.inert : false,
        winInert: win instanceof HTMLElement ? win.inert : false,
      };
    }, form);
    if (state.form !== form || state.cardVisibility !== "hidden" || state.armed) {
      throw new Error(`${form} still exposes the card ${JSON.stringify(state)}`);
    }
    if (form === "companion" && !state.streamInert) {
      throw new Error(`companion collapse is not inert ${JSON.stringify(state)}`);
    }
    if (form === "pill" && !state.winInert) {
      throw new Error(`pill window is not inert ${JSON.stringify(state)}`);
    }
    return state;
  }

  for (const form of ["pill", "companion"]) {
    const point = await approvePoint();
    await page.evaluate((next) => window.__shellStage.snap(next), form);
    await collapsed(form);
    await page.mouse.click(point.x, point.y);
    await page.waitForTimeout(400);
    assertQuiet(`${form} blind click`);

    await page.evaluate((next) => window.__shellStage.snap(next), form);
    await collapsed(form);
    await page.keyboard.press("Alt+ArrowDown");
    await page.waitForTimeout(200);
    await page.keyboard.down("Control");
    await page.keyboard.down("Enter");
    await page.waitForTimeout(1000);
    await page.keyboard.up("Enter");
    await page.keyboard.up("Control");
    const afterKey = await collapsed(form);
    if (afterKey.focused) {
      throw new Error(`${form} Alt+Down focused the hidden card`);
    }
    assertQuiet(`${form} Alt+Down hold`);
  }

  async function armCard() {
    await page.evaluate(() => {
      delete document.visibilityState;
      window.__shellStage.snap("full");
      const composer = document.querySelector(".composer");
      if (composer instanceof HTMLElement) {
        composer.style.display = "none";
      }
      window.dispatchEvent(new Event("focus"));
      document.dispatchEvent(new Event("visibilitychange"));
      const article = document.querySelector("article.card");
      if (article instanceof HTMLElement) {
        article.focus();
      }
    });
    await page.waitForFunction(() => document.querySelector("article.card .hold-hint.armed"), null, { timeout: 5000 });
  }

  async function holdStarted() {
    await page.keyboard.down("Control");
    await page.keyboard.down("Enter");
    await page.waitForTimeout(180);
    const started = await page.evaluate(() => Boolean(document.querySelector("article.card .check")));
    if (!started) {
      throw new Error("hold did not start on the focused card");
    }
  }

  async function assertReset(label) {
    const state = await page.evaluate(() => {
      const article = document.querySelector("article.card");
      return {
        check: Boolean(article?.querySelector("svg.check:not(.inline)")),
        armed: Boolean(article?.querySelector(".hold-hint.armed")),
        focused: document.activeElement === article || Boolean(article?.contains(document.activeElement)),
      };
    });
    if (state.check || state.armed || state.focused) {
      throw new Error(`${label} left the hold running ${JSON.stringify(state)}`);
    }
  }

  async function releaseAndExpectReset(label) {
    await page.evaluate(() => {
      window.__shellStage.snap("full");
      delete document.visibilityState;
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("focus"));
      const article = document.querySelector("article.card");
      if (article instanceof HTMLElement) {
        article.focus();
      }
    });
    const kept = await page.evaluate(() => Boolean(document.querySelector("article.card .hold-hint.armed")));
    if (kept) {
      throw new Error(`${label} kept seen armed across the sight loss`);
    }
    await page.waitForTimeout(650);
    const early = await page.evaluate(() => Boolean(document.querySelector("article.card .hold-hint.armed")));
    if (early) {
      throw new Error(`${label} re-armed without a fresh dwell`);
    }
    assertQuiet(label);
    await page.keyboard.up("Enter");
    await page.keyboard.up("Control");
    await page.waitForTimeout(250);
    assertQuiet(label);
  }

  await armCard();
  const sinkFocused = await page.evaluate(() => {
    const sink = document.createElement("button");
    sink.type = "button";
    sink.id = "focus-sink";
    sink.textContent = "sink";
    document.body.appendChild(sink);
    sink.focus();
    return document.activeElement === sink;
  });
  if (!sinkFocused) {
    throw new Error("could not move focus off the card");
  }
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(1000);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  assertQuiet("window-level hold");

  await armCard();
  await holdStarted();
  await page.evaluate(() => window.__shellStage.snap("pill"));
  await collapsed("pill");
  await assertReset("pill mid-hold");
  await releaseAndExpectReset("pill mid-hold resume");

  await armCard();
  await holdStarted();
  await page.evaluate(() => window.__shellStage.snap("companion"));
  await collapsed("companion");
  await assertReset("companion mid-hold");
  await releaseAndExpectReset("companion mid-hold resume");

  await armCard();
  await holdStarted();
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await assertReset("window blur mid-hold");
  await releaseAndExpectReset("window blur resume");

  await armCard();
  await holdStarted();
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await assertReset("visibility hidden mid-hold");
  await releaseAndExpectReset("visibility hidden resume");

  await page.evaluate(() => {
    const composer = document.querySelector(".composer");
    if (composer instanceof HTMLElement) {
      composer.style.display = "";
    }
    delete document.visibilityState;
    window.dispatchEvent(new Event("focus"));
    window.__shellStage.snap("pill");
  });
  await collapsed("pill");
  assertQuiet("before 1 waiting");

  await page.locator(".cl.pill .ph").click();
  await page.waitForTimeout(400);
  await collapsed("pill");
  assertQuiet("Ask Reviewer");

  await page.getByRole("button", { name: "1 waiting" }).click();
  await page.waitForFunction(() => {
    const win = document.querySelector(".win");
    const article = document.querySelector("article.card");
    if (!(win instanceof HTMLElement) || !article) {
      return false;
    }
    const cardStyle = getComputedStyle(article);
    const winStyle = getComputedStyle(win);
    return (
      win.dataset.shellForm === "full" &&
      !win.inert &&
      cardStyle.visibility !== "hidden" &&
      cardStyle.display !== "none" &&
      Number.parseFloat(winStyle.opacity) > 0.9
    );
  }, null, { timeout: 4000 });

  const opened = await page.evaluate(() => {
    const article = document.querySelector("article.card");
    return {
      hello: article?.querySelector(".draft")?.textContent ?? "",
      armed: Boolean(article?.querySelector(".hold-hint.armed")),
    };
  });
  if (!opened.hello.includes("Hello")) {
    throw new Error(`opened card does not show Hello ${JSON.stringify(opened)}`);
  }
  if (opened.armed) {
    throw new Error("seen lock was already armed when the card opened");
  }

  await page.evaluate(() => {
    document.querySelector("article.card button.approve")?.click();
  });
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(350);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  assertQuiet("shortcut before seen lock");

  await page.keyboard.press("Alt+ArrowDown");
  try {
    await page.waitForFunction(
      () => {
        const article = document.querySelector("article.card");
        return Boolean(article && document.activeElement === article && article.querySelector(".hold-hint.armed"));
      },
      null,
      { timeout: 4000 },
    );
  } catch (error) {
    const sight = await page.evaluate(() => {
      const article = document.querySelector("article.card");
      const actions = article?.querySelector(".actions");
      const box = actions?.getBoundingClientRect();
      const hit = box ? document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) : null;
      const evidence = article?.querySelector(".evidence");
      const cardBox = article?.getBoundingClientRect();
      const evidenceBox = evidence?.getBoundingClientRect();
      return {
        focused: document.activeElement === article,
        active: document.activeElement?.className ?? "",
        hint: article?.querySelector(".hold-hint")?.textContent ?? "",
        hit: hit ? `${hit.tagName}.${hit.className}` : "",
        card: cardBox ? { y: cardBox.y, h: cardBox.height, bottom: cardBox.bottom } : null,
        evidence: evidenceBox
          ? { y: evidenceBox.y, h: evidenceBox.height, bottom: evidenceBox.bottom }
          : null,
        innerHeight: window.innerHeight,
      };
    });
    throw new Error(`seen lock did not arm ${JSON.stringify(sight)}`, { cause: error });
  }
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.waitForTimeout(900);
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  await page.waitForFunction(() => (document.querySelector("article.card")?.textContent ?? "").includes("Approved"), null, {
    timeout: 4000,
  });
  if (decisions.length !== 1 || decisions[0].body.decision !== "approve" || !decisions[0].path.endsWith("/decision")) {
    throw new Error(`hold did not approve once ${JSON.stringify(decisions)}`);
  }
  const receipt = await page.evaluate(() => document.querySelector("article.card")?.textContent ?? "");
  const eventId = receipt.match(/ev_[0-9a-f]{6}/i)?.[0] ?? "";
  if (!/^ev_[0-9a-f]{6}$/i.test(eventId)) {
    throw new Error(`decision event id is not ev_ plus 6 hex: ${receipt}`);
  }

  await page.evaluate(() => window.__shellStage.snap("companion"));
  await page.keyboard.down("Control");
  await page.keyboard.down("z");
  await page.keyboard.up("z");
  await page.keyboard.up("Control");
  await page.waitForTimeout(200);
  if (undos.length !== 0) {
    throw new Error(`hidden companion undo posted ${JSON.stringify(undos)}`);
  }
  await page.evaluate(() => {
    window.__shellStage.snap("full");
    const sink = document.createElement("button");
    sink.type = "button";
    sink.id = "undo-sink";
    sink.textContent = "sink";
    document.body.appendChild(sink);
    sink.focus();
  });
  await page.keyboard.down("Control");
  await page.keyboard.down("z");
  await page.keyboard.up("z");
  await page.keyboard.up("Control");
  await page.waitForTimeout(200);
  if (undos.length !== 0) {
    throw new Error(`unfocused undo posted ${JSON.stringify(undos)}`);
  }
  await page.locator("article.card").focus();
  await page.keyboard.down("Control");
  await page.keyboard.down("z");
  await page.keyboard.up("z");
  await page.keyboard.up("Control");
  await page.waitForFunction(() => (document.querySelector("article.card button.approve")?.textContent ?? "").includes("Approve draft"), null, {
    timeout: 4000,
  });
  if (undos.length !== 1 || !undos[0].path.endsWith("/undo")) {
    throw new Error(`undo did not post ${JSON.stringify(undos)}`);
  }
  if (decisions.length !== 1) {
    throw new Error(`undo sent another decision ${JSON.stringify(decisions)}`);
  }
  console.log("pill-approval-smoke: pass");
} finally {
  await browser.close();
  preview.kill("SIGTERM");
}
