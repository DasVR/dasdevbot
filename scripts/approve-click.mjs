/**
 * Focus the pending card, then hold Approve for the 600ms ink fill.
 * A one-frame click must not post. The hold hint stays in the layout
 * (visibility hidden) so the button does not move between pointerdown and
 * pointerup. After the hold, Windows Hello is the platform authenticator.
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

function launchBrowser() {
  const channel = process.env.FONT_PROOF_CHANNEL;
  if (channel) {
    return chromium.launch({ channel });
  }
  return chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
}

const now = Date.UTC(2026, 8, 30, 13, 4, 0);

function approval(status) {
  return {
    id: "ap_click",
    job_id: "job_click",
    agent_id: "reviewer",
    agent_name: "Reviewer",
    thread_id: "thread_click",
    effect_class: "external",
    action: "post_pr_comment",
    purpose: "Leave one review comment flagging an unhandled error path in the session handoff.",
    draft:
      "If refresh() rejects on a 401, the handoff lock is never released. Wrap it in try/finally so the next session can take the lock.",
    evidence: {
      repo: "DasVR/NIL",
      pr: "#212 handoff: release lock on refresh",
      pr_number: "212",
      ref: "phase0 @ a41c9e2",
      event_id: "ev_abcdef",
      kind: "repo.push",
    },
    evidence_text: "repo DasVR/NIL\npr #212 handoff: release lock on refresh\nref phase0\nevent ev_abcdef",
    status,
    provider: "mock",
    model: "reviewer-small",
    usage_kind: "estimated",
    input_tokens: 2418,
    output_tokens: 212,
    micro_usd: 431,
    created_at: now,
    expires_at: null,
    decided_at: status === "pending" ? null : now,
    decision_event_id: status === "pending" ? null : "ev_0142",
    reason: null,
    committed: false,
    undo_until: status === "pending" ? null : Date.now() + 60_000,
  };
}

function snapshot(status) {
  const card = approval(status);
  return {
    protocol: 1,
    role: "server",
    node: "approve-click",
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
    approvals: [card],
    ledger: [],
    events: [
      {
        id: "ev_push",
        version: 1,
        hlc: "1759241040000:1:node",
        source: "demo",
        kind: "repo.push",
        thread_id: "thread_click",
        idempotency_key: "push-1",
      },
      {
        id: "ev_ask",
        version: 1,
        hlc: "1759241041000:0:node",
        source: "demo",
        kind: "approval.requested",
        thread_id: "thread_click",
        idempotency_key: `approval-requested:${card.id}`,
      },
    ],
  };
}

const port = await freePort();
const origin = `http://localhost:${port}`;
const preview = spawn(
  process.execPath,
  [viteBin, "preview", "--host", "localhost", "--port", String(port), "--strictPort"],
  { cwd: desktop, stdio: ["ignore", "pipe", "pipe"] },
);
drain(preview.stdout);
drain(preview.stderr);

let browser;
try {
  await waitForHttp(origin, preview);
  browser = await launchBrowser();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1700 } });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  await page.addInitScript(() => {
    const credentialsApi = navigator.credentials;
    const create = credentialsApi.create.bind(credentialsApi);
    credentialsApi.create = async (options) => {
      window.__helloCalls = (window.__helloCalls ?? 0) + 1;
      return create(options);
    };
  });
  let phase = "pending";
  let posted = null;
  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    const method = route.request().method();
    if (url.pathname === "/v1/snapshot" && method === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(snapshot(phase)),
      });
      return;
    }
    if (url.pathname === "/v1/approvals/ap_click/decision" && method === "POST") {
      posted = route.request().postDataJSON();
      if (posted?.decision === "approve") {
        phase = "approved";
      }
      await route.fulfill({ status: 200, contentType: "application/json", body: "{}" });
      return;
    }
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });

  await page.goto(origin, { waitUntil: "networkidle" });
  const card = page.locator("article.card");
  await card.waitFor();
  if ((await page.locator(".face-done").count()) !== 0) {
    throw new Error(".face-done is still in the card");
  }
  await card.focus();
  const button = page.getByRole("button", { name: "Approve draft" });
  const before = await button.boundingBox();
  if (!before) {
    throw new Error("Approve draft has no box");
  }
  const hint = page.locator(".hold-hint");
  const hintBefore = await hint.boundingBox();
  if (!hintBefore || hintBefore.height < 10) {
    throw new Error("hold hint does not reserve a line before the click");
  }
  const point = { x: before.x + before.width / 2, y: before.y + before.height / 2 };
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve())));
  const during = await button.boundingBox();
  const hintDuring = await hint.boundingBox();
  const visibility = await hint.evaluate((el) => getComputedStyle(el).visibility);
  await page.mouse.up();
  // The press squash moves the box a few pixels. The hint-unmount bug moved it ~24px.
  if (!during || Math.abs(during.y - before.y) > 8 || Math.abs(during.x - before.x) > 8) {
    throw new Error(`Approve moved on mousedown: ${JSON.stringify(before)} -> ${JSON.stringify(during)}`);
  }
  if (visibility !== "hidden") {
    throw new Error(`hold hint stayed ${visibility} after the card lost focus`);
  }
  if (!hintDuring || Math.abs(hintDuring.height - hintBefore.height) > 1) {
    throw new Error("hold hint dropped its box when the card lost focus");
  }
  await page.waitForTimeout(400);
  if (posted !== null) {
    throw new Error("a one-frame click posted a decision");
  }
  if ((await page.locator("article.card .receipt").count()) !== 0) {
    throw new Error("a one-frame click filed the receipt");
  }
  await page.locator(".hold-hint.armed").waitFor({ state: "attached", timeout: 4000 });
  const held = await button.boundingBox();
  if (!held) {
    throw new Error("Approve draft left the card before the hold");
  }
  await page.mouse.move(held.x + held.width / 2, held.y + held.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(800);
  await page.mouse.up();
  await page.locator("article.card .receipt b.approved", { hasText: "Approved" }).waitFor({ timeout: 8000 });
  const helloCalls = await page.evaluate(() => window.__helloCalls ?? 0);
  if (helloCalls < 1) {
    throw new Error("the hold committed without calling the platform authenticator");
  }
  if ((await page.getByRole("button", { name: "Confirm" }).count()) !== 0) {
    throw new Error("an in-app Confirm button was drawn for Windows Hello");
  }
  if (posted?.decision !== "approve") {
    throw new Error(`decision was ${posted?.decision ?? "not posted"}`);
  }
  if ((await page.locator(".face-done").count()) !== 0) {
    throw new Error(".face-done appeared after approve");
  }
  console.log("approve-click: pass");
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
