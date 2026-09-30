/**
 * Proves visible text uses the bundled faces.
 * - font requests stay on this origin (the vendored woff2 files)
 * - every visible text node, in each captured state, is painted by a bundled
 *   face with isCustomFont true
 *
 * A machine-wide Figtree install is not enough: isCustomFont must be true.
 * Onest is the token fallback. This script does not probe it on its own.
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

const port = process.env.FONT_PROOF_PORT ? Number(process.env.FONT_PROOF_PORT) : await freePort();
const origin = `http://127.0.0.1:${port}`;

const blockedHosts = ["fonts.googleapis.com", "fonts.gstatic.com", "cdn.jsdelivr.net", "fontsource.org"];

function launchBrowser() {
  const channel = process.env.FONT_PROOF_CHANNEL;
  if (channel) {
    return chromium.launch({ channel });
  }
  return chromium.launch({ channel: "chrome" }).catch(() => chromium.launch());
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

const preview = spawn(
  process.execPath,
  [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: desktop,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
drain(preview.stdout);
drain(preview.stderr);

const bundledName = /figtree|onest|jetbrains\s*mono/i;

function isBundledFace(font) {
  const name = `${font.familyName} ${font.postScriptName ?? ""}`;
  return bundledName.test(name) && font.isCustomFont === true;
}

function idleSnapshot() {
  return snapshot({ status: "idle", approvals: [], events: [], ledger: [] });
}

function streamSnapshot() {
  return snapshot({
    status: "idle",
    approvals: [],
    events: [eventRow("ev_note", "session.note", "note-1", "1759241040000:0:node")],
    ledger: [],
  });
}

function waitingSnapshot() {
  const card = approval("pending");
  return snapshot({
    status: "blocked",
    approvals: [card],
    events: [
      eventRow("ev_push", "repo.push", "push-1", "1759241040000:1:node"),
      eventRow("ev_ask", "approval.requested", `approval-requested:${card.id}`, "1759241041000:0:node"),
    ],
    ledger: [ledgerLine()],
  });
}

function receiptSnapshot() {
  const card = approval("approved");
  return snapshot({
    status: "blocked",
    approvals: [card],
    events: [
      eventRow("ev_push", "repo.push", "push-1", "1759241040000:1:node"),
      eventRow("ev_ask", "approval.requested", `approval-requested:${card.id}`, "1759241041000:0:node"),
    ],
    ledger: [ledgerLine()],
  });
}

function snapshot({ status, approvals, events, ledger }) {
  return {
    protocol: 1,
    role: "server",
    node: "font-proof",
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
        status,
      },
    ],
    approvals,
    ledger,
    events,
  };
}

function approval(status) {
  const now = Date.UTC(2026, 8, 30, 13, 4, 0);
  return {
    id: "ap_font",
    job_id: "job_font",
    agent_id: "reviewer",
    agent_name: "Reviewer",
    thread_id: "thread_font",
    effect_class: "external",
    action: "post_pr_comment",
    purpose: "Leave one review comment flagging an unhandled error path in the session handoff.",
    draft:
      "If refresh() rejects on a 401, the handoff lock is never released. Wrap it in try/finally so the next session can take the lock.",
    evidence: {
      repo: "DasVR/NIL",
      pr: "#212 handoff: release lock on refresh",
      pr_number: "212",
      ref: "phase0",
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

function eventRow(id, kind, key, hlc) {
  return {
    id,
    version: 1,
    hlc,
    source: "demo",
    kind,
    thread_id: "thread_font",
    idempotency_key: key,
  };
}

function ledgerLine() {
  return {
    id: "led_font",
    agent_id: "reviewer",
    agent_name: "Reviewer",
    project: "DasVR/NIL",
    provider: "mock",
    model: "reviewer-small",
    usage_kind: "estimated",
    input_tokens: 2418,
    output_tokens: 212,
    micro_usd: 431,
    note: "estimated",
  };
}

async function markTextNodes(page) {
  return page.evaluate(async () => {
    await document.fonts.ready;
    document.querySelectorAll("[data-font-audit]").forEach((node) => {
      node.removeAttribute("data-font-audit");
    });
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    const ids = [];
    let index = 0;
    while (walker.nextNode()) {
      const text = walker.currentNode.nodeValue ?? "";
      if (text.trim().length === 0) {
        continue;
      }
      const el = walker.currentNode.parentElement;
      if (!el) {
        continue;
      }
      const style = getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
        continue;
      }
      const rect = el.getBoundingClientRect();
      if (rect.width < 0.5 || rect.height < 0.5) {
        continue;
      }
      let id = el.getAttribute("data-font-audit");
      if (!id) {
        id = `n${index}`;
        index += 1;
        el.setAttribute("data-font-audit", id);
        ids.push({ id, text: text.trim().slice(0, 80) });
      }
    }
    return ids;
  });
}

async function auditState(page, session, state) {
  const nodes = await markTextNodes(page);
  const { root } = await session.send("DOM.getDocument");
  const faceGlyphs = new Map();
  const failures = [];
  for (const node of nodes) {
    const { nodeId } = await session.send("DOM.querySelector", {
      nodeId: root.nodeId,
      selector: `[data-font-audit="${node.id}"]`,
    });
    if (!nodeId) {
      failures.push(`${state}: missing audit node for “${node.text}”`);
      continue;
    }
    const platform = await session.send("CSS.getPlatformFontsForNode", { nodeId });
    const fonts = platform.fonts ?? [];
    if (fonts.length === 0) {
      failures.push(`${state}: no platform fonts for “${node.text}”`);
      continue;
    }
    for (const font of fonts) {
      const key = font.familyName || font.postScriptName || "unknown";
      faceGlyphs.set(key, (faceGlyphs.get(key) ?? 0) + (font.glyphCount ?? 0));
      if (!isBundledFace(font)) {
        const name = `${font.familyName} / ${font.postScriptName ?? ""} / custom ${font.isCustomFont}`;
        failures.push(`${state}: “${node.text}” uses ${name}`);
      }
    }
  }
  return { state, textNodes: nodes.length, faceGlyphs, failures };
}

let browser;
try {
  await waitForHttp(origin, preview);
  browser = await launchBrowser();
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36",
  });
  const page = await context.newPage();
  const fontRequests = [];
  page.on("request", (request) => {
    const url = request.url();
    const type = request.resourceType();
    if (type === "font" || /\.woff2(\?|$)/.test(url)) {
      fontRequests.push(url);
    }
  });

  let fixture = idleSnapshot();
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
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });

  const session = await context.newCDPSession(page);
  await session.send("DOM.enable");
  await session.send("CSS.enable");

  const states = [
    ["idle", idleSnapshot(), async () => page.getByText("Nothing is waiting.").waitFor()],
    ["stream", streamSnapshot(), async () => page.locator(".stream .row").first().waitFor()],
    [
      "waiting",
      waitingSnapshot(),
      async () => {
        const card = page.locator("article.card");
        await card.waitFor();
        await card.focus();
        await page.locator(".hold-hint").getByText("unlocks once the evidence has been on screen").waitFor();
      },
    ],
    [
      "waiting-armed",
      waitingSnapshot(),
      async () => {
        const card = page.locator("article.card");
        await card.scrollIntoViewIfNeeded();
        await card.focus();
        await page.locator(".hold-hint.armed").waitFor({ timeout: 4000 });
      },
    ],
    ["receipt", receiptSnapshot(), async () => page.locator("article.card .receipt").waitFor()],
  ];

  const audits = [];
  for (const [name, next, ready] of states) {
    fixture = next;
    await page.goto(origin, { waitUntil: "networkidle" });
    await page.locator(".wordmark").waitFor();
    await ready();
    audits.push(await auditState(page, session, name));
  }

  await page.goto(`${origin}/gallery.html`, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "Line icons" }).waitFor();
  audits.push(await auditState(page, session, "gallery"));

  const remote = fontRequests.filter((url) => {
    const host = new URL(url).host;
    return host !== new URL(origin).host || blockedHosts.some((blocked) => host.endsWith(blocked));
  });
  const totals = new Map();
  let textNodes = 0;
  const failures = [];
  if (fontRequests.length === 0) {
    failures.push("no font requests were observed");
  }
  if (remote.length > 0) {
    failures.push(`font requests left the origin: ${remote.join(", ")}`);
  }
  for (const audit of audits) {
    textNodes += audit.textNodes;
    failures.push(...audit.failures);
    for (const [face, glyphs] of audit.faceGlyphs) {
      totals.set(face, (totals.get(face) ?? 0) + glyphs);
    }
  }
  const faces = [...totals.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  console.log(`text nodes: ${textNodes}`);
  for (const audit of audits) {
    const parts = [...audit.faceGlyphs.entries()]
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([face, glyphs]) => `${face} ${glyphs}`)
      .join(", ");
    console.log(`${audit.state}: ${audit.textNodes} text nodes${parts ? ` (${parts})` : ""}`);
  }
  console.log("faces:");
  for (const [face, glyphs] of faces) {
    console.log(`  ${face}: ${glyphs} glyphs`);
  }
  if (!faces.some(([face]) => /figtree/i.test(face))) {
    failures.push("no visible text used the bundled Figtree face");
  }
  if (!faces.some(([face]) => /jetbrains/i.test(face))) {
    failures.push("no visible text used the bundled JetBrains Mono face");
  }
  if (failures.length > 0) {
    throw new Error(failures.join("\n"));
  }
  console.log("font-proof: pass");
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
