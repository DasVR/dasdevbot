/**
 * No lease label reaches the DOM.
 * The stage roster and a live snapshot that carries a lease token and an epoch
 * must not put the word "lease", a job id, the token or the epoch into text, a
 * tooltip, a title, or an aria label.
 * A running teammate shows the mock's sub with the lease label replaced by its
 * state: "running · 2m14s · 208 / 8000 tok" on the stage, live elapsed and
 * spent / cap tokens on a snapshot (CD ruling 2, Oct 1 8:38 PM).
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import net from "node:net";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));

const LEASE_TOKEN = "lst_TEST_FIXTURE_NOT_A_LEASE";
const EPOCH = "773341";
const SPENT = "918273";
const CAP = "645120";
const FORBIDDEN = [
  LEASE_TOKEN,
  EPOCH,
  "tokens_spent",
  "token_cap",
  "bld_02",
  "rev_01",
  "dep_03",
];
/** The word itself, as a label. "release" does not match. */
const LEASE_WORD = /\blease\b/i;

function assertClean(where, dom) {
  for (const secret of FORBIDDEN) {
    if (dom.includes(secret)) {
      throw new Error(`${where} DOM contains ${secret}`);
    }
  }
  const word = dom.match(new RegExp(`.{0,40}${LEASE_WORD.source}.{0,40}`, "i"));
  if (word) {
    throw new Error(`${where} DOM has a lease label: ${word[0]}`);
  }
}

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

const snapshot = {
  protocol: 1,
  role: "server",
  node: "lease-smoke",
  provider: "mock",
  provider_detail: "mock",
  sync: "stub",
  endpoint_id: null,
  agents: [
    {
      id: "builder",
      name: "Builder",
      project: "DasVR/NIL",
      persona: "Builds the branch.",
      token_cap: Number(CAP),
      tokens_spent: Number(SPENT),
      status: "working",
      lease_token: LEASE_TOKEN,
      epoch: Number(EPOCH),
    },
  ],
  approvals: [],
  ledger: [],
  events: [],
};

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
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

function surface() {
  const parts = [];
  const visit = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      parts.push(node.textContent ?? "");
      return;
    }
    if (!(node instanceof Element)) {
      return;
    }
    for (const attr of node.attributes) {
      parts.push(attr.name, attr.value);
    }
    for (const child of node.childNodes) {
      visit(child);
    }
  };
  visit(document.documentElement);
  return parts.join("\n");
}

await waitForHttp(origin);
const browser = await launchBrowser();
try {
  const page = await browser.newPage();
  await page.goto(`${origin}/?shellStage=1`, { waitUntil: "networkidle" });
  await page.locator(".wordmark").waitFor();
  const stage = await page.evaluate(surface);
  assertClean("stage", stage);
  if (!stage.includes("running · 2m14s · 208 / 8000 tok")) {
    throw new Error("stage roster dropped Builder's running state");
  }

  await page.route("**/v1/snapshot", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(snapshot),
    }),
  );
  await page.goto(`${origin}/`, { waitUntil: "networkidle" });
  await page.locator(".wordmark").waitFor();
  await page.waitForFunction(
    ({ spent, cap }) =>
      new RegExp(`^running · \\d+m\\d\\ds · ${spent} / ${cap} tok$`).test(document.querySelector(".roster .sub")?.textContent ?? ""),
    { spent: SPENT, cap: CAP },
    { timeout: 10000 },
  );
  const first = await page.locator(".roster .sub").first().textContent();
  await page.waitForTimeout(2200);
  if ((await page.locator(".roster .sub").first().textContent()) === first) {
    throw new Error(`the running timer is frozen at ${first}`);
  }
  const live = await page.evaluate(surface);
  assertClean("live", live);
  console.log("lease-dom-smoke: pass");
} finally {
  await browser.close();
  preview.kill("SIGTERM");
}
