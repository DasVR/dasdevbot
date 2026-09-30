/**
 * Captures the nine approval-queue acceptance checks.
 * Stills are 1280×800 at 2×, timezone America/New_York, pointer parked off the rows.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import net from "node:net";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));
const outDir = fileURLToPath(new URL("../docs/review/queue/", import.meta.url));

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

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;

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
  { cwd: desktop, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, TZ: "America/New_York" } },
);
drain(preview.stdout);
drain(preview.stderr);

const T_1114 = Date.UTC(2026, 8, 30, 15, 14);
const T_1140 = Date.UTC(2026, 8, 30, 15, 40);
const T_1152 = Date.UTC(2026, 8, 30, 15, 52);

function agent(id, name) {
  return {
    id,
    name,
    project: "DasVR/NIL",
    persona: "Reviews the branch before anything is posted.",
    token_cap: 20000,
    tokens_spent: 1280,
    status: "blocked",
  };
}

function approval(overrides) {
  const now = Date.now();
  return {
    id: "ap_old",
    job_id: "job_q",
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
    created_at: T_1114,
    expires_at: now + 15 * 60 * 1000,
    decided_at: null,
    decision_event_id: null,
    reason: null,
    committed: false,
    undo_until: null,
    ...overrides,
  };
}

function eventRow(id, key, hlc) {
  return {
    id,
    version: 1,
    hlc,
    source: "demo",
    kind: "approval.requested",
    thread_id: "thread_reviewer",
    idempotency_key: key,
  };
}

function fillers(count) {
  const rows = [];
  for (let index = 0; index < count; index += 1) {
    rows.push(eventRow(`ev_fill_${index}`, `fill-${index}`, `${T_1114 - (index + 1) * 60_000}:0:node`));
  }
  return rows;
}

function baseSnapshot(approvals, events) {
  return {
    protocol: 1,
    role: "server",
    node: "queue-proof",
    provider: "mock",
    provider_detail: "mock",
    sync: "stub",
    agents: [agent("reviewer", "Reviewer"), agent("builder", "Builder")],
    approvals,
    ledger: [],
    events,
  };
}

function queueFixture(expiresInMs) {
  const now = Date.now();
  const oldest = approval({
    id: "ap_old",
    created_at: T_1114,
    expires_at: now + expiresInMs,
    evidence: { repo: "DasVR/NIL", ref: "212", event_id: "ev_old", kind: "repo.push" },
  });
  const middle = approval({
    id: "ap_mid",
    created_at: T_1140,
    expires_at: now + 15 * 60 * 1000,
    action: "open_issue",
    evidence: { repo: "DasVR/NIL", ref: "214", event_id: "ev_mid", kind: "repo.push" },
  });
  const newest = approval({
    id: "ap_new",
    agent_id: "builder",
    agent_name: "Builder",
    thread_id: "thread_builder",
    created_at: T_1152,
    expires_at: now + 15 * 60 * 1000,
    action: "post_status",
    evidence: { repo: "DasVR/NIL", ref: "218", event_id: "ev_new", kind: "repo.push" },
  });
  const destructive = approval({
    id: "ap_dest",
    effect_class: "destructive",
    action: "force_push",
    draft: "git push --force origin phase0",
    created_at: T_1140 + 1000,
    expires_at: now + 15 * 60 * 1000,
    evidence: { repo: "DasVR/NIL", ref: "main", event_id: "ev_dest", kind: "repo.push" },
  });
  return baseSnapshot(
    [newest, destructive, middle, oldest],
    [
      eventRow("ev_old", "approval-requested:ap_old", `${T_1114}:0:node`),
      eventRow("ev_mid", "approval-requested:ap_mid", `${T_1140}:0:node`),
      eventRow("ev_new", "approval-requested:ap_new", `${T_1152}:0:node`),
      eventRow("ev_dest", "approval-requested:ap_dest", `${T_1140 + 1000}:0:node`),
      ...fillers(14),
    ],
  );
}

let mode = "queue";
let failSnapshot = false;
let fixture = queueFixture(111_200);

const previewReady = waitForHttp(origin, preview);
let browser;
const checks = [];

function record(id, result, evidence, extra = {}) {
  checks.push({ id, result, evidence, ...extra });
  console.log(`check ${id}: ${result} — ${evidence}`);
}

async function shot(page, name) {
  await page.mouse.move(2, 2);
  await page.screenshot({ path: `${outDir}${name}.png`, animations: "disabled" });
}

async function rowIds(page) {
  return page.locator("[data-queue-rows] [data-queue-row]").evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute("data-approval-id")),
  );
}

async function openSheet(page) {
  await page.locator(".review-link").click();
  await page.locator("[data-review-sheet]").waitFor();
}

async function pressAltDown(page, expected) {
  await page.keyboard.press("Alt+ArrowDown");
  if (!expected) {
    return;
  }
  await page.waitForFunction(
    (id) => document.activeElement?.getAttribute("data-approval-id") === id,
    expected,
    { timeout: 3000 },
  );
}

try {
  await previewReady;
  await mkdir(outDir, { recursive: true });
  browser = await chromium.launch({ channel: "chrome" });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    timezoneId: "America/New_York",
  });
  const page = await context.newPage();
  const logs = [];
  page.on("console", (message) => {
    if (message.type() === "error" && !message.text().includes("status of 500")) {
      logs.push(message.text());
    }
  });
  page.on("pageerror", (error) => {
    logs.push(String(error));
  });

  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
      if (failSnapshot) {
        await route.fulfill({ status: 500, contentType: "text/plain", body: "lost" });
        return;
      }
      if (mode === "queue") {
        fixture = queueFixture(111_200);
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(fixture),
      });
      return;
    }
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });

  mode = "queue";
  failSnapshot = false;
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.locator(".wordmark").waitFor();
  await page.locator("article.card").waitFor();
  await page.locator("article.card").focus();
  await page.locator(".hold-hint.armed").waitFor({ timeout: 5000 });
  const armedBeforeSheet = await page.locator("article.card").getAttribute("data-seen");

  await openSheet(page);
  const seenWhileOpen = await page.locator("article.card").getAttribute("data-seen");
  const sinceWhileOpen = await page.locator("article.card").getAttribute("data-seen-since");
  await page.waitForTimeout(1200);
  const seenAfterDwell = await page.locator("article.card").getAttribute("data-seen");
  const sinceAfterDwell = await page.locator("article.card").getAttribute("data-seen-since");

  const order = await rowIds(page);
  const metas = await page.locator("[data-queue-rows] .meta").allTextContents();
  const expiresLabel = metas[0]?.trim() ?? "";
  await page.waitForTimeout(2000);
  const orderLater = await rowIds(page);
  const expiresLater = (await page.locator("[data-queue-rows] .meta").first().textContent())?.trim() ?? "";
  const waitingCount = await page.locator(".review-link").getAttribute("data-waiting-count");
  const dotCountOpen = await page.locator("[data-waiting-dot]").count();
  const trayText = (await page.locator(".tray-num").textContent())?.trim() ?? "";
  const destructiveInSheet = await page.locator("[data-review-sheet]").getByText("Destructive actions are off in this build.").count();
  const checkboxCount = await page.locator("[data-review-sheet] input[type=checkbox]").count();
  const approveAll = await page.getByText(/approve all/i).count();
  const colors = await page.evaluate(() => {
    const tray = getComputedStyle(document.querySelector(".tray-num"));
    const ink = getComputedStyle(document.querySelector(".wordmark strong"));
    const dot = document.querySelector("[data-review-sheet] [data-waiting-dot]");
    const dotStyle = dot ? getComputedStyle(dot) : null;
    return {
      trayColor: tray.color,
      trayBg: tray.backgroundColor,
      inkColor: ink.color,
      dotBg: dotStyle ? dotStyle.backgroundColor : "",
    };
  });
  await shot(page, "check-01");

  const clickStamp = await page.evaluate(() => performance.now());
  await page.locator('[data-queue-row][data-approval-id="ap_new"]').click();
  await page.waitForFunction(() => document.querySelector(".well")?.getAttribute("data-sheet") === "closed");
  await page.waitForFunction(() => document.querySelector('article[data-approval-id="ap_new"]')?.getAttribute("data-seen") === "arming", null, {
    timeout: 4000,
  });
  const landing = await page.evaluate(() => {
    const card = document.querySelector('article[data-approval-id="ap_new"]');
    const source = document.querySelector('[data-source-for="ap_new"]');
    const well = document.querySelector(".well");
    const since = Number(card?.getAttribute("data-seen-since") ?? "NaN");
    const landed = Number(card?.getAttribute("data-landed-at") ?? "NaN");
    const cardBox = card?.getBoundingClientRect();
    const sourceBox = source?.getBoundingClientRect();
    return {
      since,
      landed,
      seen: card?.getAttribute("data-seen") ?? "",
      agent: well?.getAttribute("data-thread-agent") ?? "",
      switchAt: Number(well?.getAttribute("data-switch-at") ?? "NaN"),
      focusAt: Number(well?.getAttribute("data-focus-at") ?? "NaN"),
      active: document.activeElement?.getAttribute("data-approval-id") ?? "",
      gap: sourceBox && cardBox ? sourceBox.bottom - (cardBox.top - 18) : null,
    };
  });
  await page.waitForTimeout(250);
  const seenMidDwell = await page.locator('article[data-approval-id="ap_new"]').getAttribute("data-seen");
  await page.waitForFunction(
    () => document.querySelector('article[data-approval-id="ap_new"]')?.getAttribute("data-seen") === "armed",
    null,
    { timeout: 2000 },
  );
  const armedAt = await page.evaluate(() => performance.now());
  await shot(page, "check-02");

  await page.locator(".review-link").focus();
  const steps = [];
  for (const expected of ["ap_old", "ap_mid", "ap_new", "ap_new"]) {
    await pressAltDown(page, expected);
    steps.push(await page.evaluate(() => document.activeElement?.getAttribute("data-approval-id") ?? ""));
  }

  const check1Pass =
    order.join(",") === "ap_old,ap_mid,ap_new" &&
    orderLater.join(",") === order.join(",") &&
    expiresLabel === "expires 1:52" &&
    expiresLater === expiresLabel &&
    steps.join(",") === "ap_old,ap_mid,ap_new,ap_new";
  record(
    1,
    check1Pass ? "PASS" : "FAIL",
    `rows ${order.join(" → ")} then ${orderLater.join(" → ")}; label "${expiresLabel}" held as "${expiresLater}"; Alt+Down ${steps.join(" → ")}`,
  );

  const dwell = armedAt - landing.since;
  const check2Pass =
    armedBeforeSheet === "armed" &&
    seenWhileOpen === "held" &&
    sinceWhileOpen == null &&
    seenAfterDwell === "held" &&
    sinceAfterDwell == null &&
    landing.seen === "arming" &&
    landing.since >= clickStamp &&
    landing.since >= landing.landed &&
    seenMidDwell === "arming" &&
    dwell >= 700 &&
    landing.agent === "builder" &&
    landing.switchAt <= landing.focusAt &&
    landing.active === "ap_new" &&
    landing.gap != null &&
    Math.abs(landing.gap) <= 8;
  record(
    2,
    check2Pass ? "PASS" : "FAIL",
    `before sheet ${armedBeforeSheet}; during sheet ${seenWhileOpen}/${seenAfterDwell} since=${sinceWhileOpen ?? "cleared"}; arming since ${Math.round(landing.since)} landed ${Math.round(landing.landed)} click ${Math.round(clickStamp)}; still ${seenMidDwell} at +250ms; armed after ${Math.round(dwell)}ms; agent ${landing.agent}; switch ${Math.round(landing.switchAt)} focus ${Math.round(landing.focusAt)}; active ${landing.active}; gap ${landing.gap == null ? "missing" : landing.gap.toFixed(2)}px`,
  );

  const check8Pass = destructiveInSheet === 0 && (await page.locator("[data-destructive]").count()) > 0;
  const policy = await page.locator("[data-destructive]").first().innerText();
  await page.locator("[data-destructive]").first().scrollIntoViewIfNeeded();
  await shot(page, "check-08");
  record(
    8,
    check8Pass && policy.includes("Destructive actions are off in this build.") ? "PASS" : "FAIL",
    `sheet destructive rows ${destructiveInSheet}; thread "${policy.replace(/\s+/g, " ").trim()}"; waiting count ${waitingCount}`,
  );

  const check9Pass =
    waitingCount === "3" &&
    String(dotCountOpen) === waitingCount &&
    trayText === waitingCount &&
    colors.trayColor === colors.inkColor &&
    (colors.trayBg === "rgba(0, 0, 0, 0)" || colors.trayBg === "transparent") &&
    colors.dotBg !== colors.trayColor;
  record(
    9,
    check9Pass ? "PASS" : "FAIL",
    `dots ${dotCountOpen} waiting ${waitingCount} tray "${trayText}"; tray ${colors.trayColor} on ${colors.trayBg}; ink ${colors.inkColor}; dot ${colors.dotBg}`,
  );

  mode = "pair";
  fixture = baseSnapshot(
    [
      approval({ id: "ap_a", created_at: T_1114, expires_at: Date.now() + 15 * 60 * 1000 }),
      approval({ id: "ap_b", created_at: T_1152, expires_at: Date.now() + 15 * 60 * 1000, action: "open_issue" }),
    ],
    [
      eventRow("ev_a", "approval-requested:ap_a", `${T_1114}:0:node`),
      eventRow("ev_b", "approval-requested:ap_b", `${T_1152}:0:node`),
    ],
  );
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.locator('article[data-approval-id="ap_a"]').waitFor();
  await openSheet(page);
  const batchControls = (await page.locator("[data-review-sheet] input[type=checkbox]").count()) + (await page.getByText(/approve all/i).count());
  const pairOrder = await rowIds(page);
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.querySelector(".well")?.getAttribute("data-sheet") === "closed");
  const decidedAt = Date.now();
  fixture = baseSnapshot(
    [
      approval({
        id: "ap_a",
        status: "approved",
        created_at: T_1114,
        decided_at: decidedAt,
        decision_event_id: "ev_0143",
        committed: false,
        undo_until: Date.now() + 60_000,
      }),
      approval({ id: "ap_b", created_at: T_1152, expires_at: Date.now() + 15 * 60 * 1000, action: "open_issue" }),
    ],
    [
      eventRow("ev_a", "approval-requested:ap_a", `${T_1114}:0:node`),
      eventRow("ev_b", "approval-requested:ap_b", `${T_1152}:0:node`),
    ],
  );
  await page.locator("article.card .undo").waitFor({ timeout: 4000 });
  const duringUndo = await page.locator(".next-waiting").count();
  const newerMountedDuringUndo = await page.locator('article[data-approval-id="ap_b"]').count();
  fixture = baseSnapshot(
    [
      approval({
        id: "ap_a",
        status: "approved",
        created_at: T_1114,
        decided_at: decidedAt,
        decision_event_id: "ev_0143",
        committed: true,
        undo_until: null,
      }),
      approval({ id: "ap_b", created_at: T_1152, expires_at: Date.now() + 15 * 60 * 1000, action: "open_issue" }),
    ],
    [
      eventRow("ev_a", "approval-requested:ap_a", `${T_1114}:0:node`),
      eventRow("ev_b", "approval-requested:ap_b", `${T_1152}:0:node`),
    ],
  );
  await page.getByText("Next waiting · Alt↓").waitFor({ timeout: 4000 });
  const newerMountedAfter = await page.locator('article[data-approval-id="ap_b"]').count();
  const focusedAfter = await page.locator("article.card").getAttribute("data-approval-id");
  await shot(page, "check-03");
  const check3Pass =
    batchControls === 0 &&
    checkboxCount === 0 &&
    approveAll === 0 &&
    pairOrder.join(",") === "ap_a,ap_b" &&
    duringUndo === 0 &&
    newerMountedDuringUndo === 0 &&
    newerMountedAfter === 0 &&
    focusedAfter === "ap_a";
  record(
    3,
    check3Pass ? "PASS" : "FAIL",
    `checkboxes/approve-all ${batchControls}; pair ${pairOrder.join(" → ")}; next-line during undo ${duringUndo}; ap_b mounted during/after ${newerMountedDuringUndo}/${newerMountedAfter}; receipt card ${focusedAfter}`,
  );

  mode = "empty";
  fixture = baseSnapshot([], []);
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.getByText("Nothing is waiting.").waitFor();
  await page.locator(".wait-entry").click();
  await page.locator("[data-review-sheet] .empty").waitFor();
  const emptyLines = await page.locator("[data-review-sheet] .empty").allTextContents();
  await shot(page, "check-04-sheet");
  await page.locator("[data-review-sheet] .close").click();
  await page.waitForFunction(() => document.querySelector(".well")?.getAttribute("data-sheet") === "closed");
  await pressAltDown(page);
  await page.locator("[data-toast]").waitFor();
  const toast = (await page.locator("[data-toast]").innerText()).replace(/\s+/g, " ").trim();
  const sheetAfterToast = await page.locator("[data-review-sheet]").count();
  await shot(page, "check-04-toast");
  const check4Pass =
    emptyLines.length === 1 &&
    emptyLines[0].trim() === "Nothing is waiting on you." &&
    toast === "Nothing is waiting on you." &&
    sheetAfterToast === 0;
  record(4, check4Pass ? "PASS" : "FAIL", `sheet lines ${JSON.stringify(emptyLines)}; toast "${toast}"; sheet after Alt+Down ${sheetAfterToast}`);

  mode = "queue";
  failSnapshot = false;
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.locator("article.card").waitFor();
  await openSheet(page);
  const rowsBeforeDrop = await rowIds(page);
  failSnapshot = true;
  await page.locator("[data-daemon-line]").waitFor({ timeout: 4000 });
  const daemonLine = (await page.locator("[data-daemon-line]").innerText()).trim();
  const rowsAfterDrop = await rowIds(page);
  await page.locator('[data-queue-row][data-approval-id="ap_old"]').click();
  await page.waitForFunction(() => document.querySelector(".well")?.getAttribute("data-sheet") === "closed");
  await page.locator('article[data-approval-id="ap_old"][data-suspended="yes"]').waitFor();
  const suspended = await page.evaluate(() => {
    const card = document.querySelector('article[data-approval-id="ap_old"]');
    const style = card ? getComputedStyle(card) : null;
    return {
      opacity: style?.opacity ?? "",
      actions: document.querySelectorAll("article.card .actions button, article.card .hello-actions button").length,
      alert: document.querySelectorAll("[role=alert]").length,
    };
  });
  await shot(page, "check-05");
  const check5Pass =
    rowsBeforeDrop.join(",") === rowsAfterDrop.join(",") &&
    daemonLine.includes("Your decisions are saved.") &&
    /^Showing what was waiting at .+\. Your decisions are saved\.$/.test(daemonLine) &&
    suspended.opacity === "0.45" &&
    suspended.actions === 0 &&
    suspended.alert === 0;
  record(
    5,
    check5Pass ? "PASS" : "FAIL",
    `rows held ${rowsAfterDrop.join(" → ")}; "${daemonLine}"; opacity ${suspended.opacity}; actions ${suspended.actions}; alerts ${suspended.alert}`,
  );

  failSnapshot = false;
  mode = "expire";
  const expireAt = Date.now() + 4500;
  fixture = baseSnapshot(
    [
      approval({ id: "ap_exp", created_at: T_1114, expires_at: expireAt }),
      approval({ id: "ap_keep", created_at: T_1152, expires_at: Date.now() + 15 * 60 * 1000, action: "open_issue" }),
    ],
    [
      eventRow("ev_exp", "approval-requested:ap_exp", `${T_1114}:0:node`),
      eventRow("ev_keep", "approval-requested:ap_keep", `${T_1152}:0:node`),
    ],
  );
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.locator("article.card").waitFor();
  await openSheet(page);
  const beforeExpire = await rowIds(page);
  await page.locator('[data-approval-id="ap_exp"][data-expired="yes"]').waitFor({ timeout: 6000 });
  const afterExpire = await rowIds(page);
  const expiredTitle = (await page.locator('[data-approval-id="ap_exp"][data-expired="yes"] .what').innerText()).trim();
  const ring = await page.evaluate(() => {
    const ringEl = document.querySelector('[data-approval-id="ap_exp"] .xring');
    if (!ringEl) {
      return "";
    }
    return getComputedStyle(ringEl).boxShadow;
  });
  await shot(page, "check-06");
  await page.locator("[data-review-sheet] .close").click();
  await page.waitForFunction(() => document.querySelector(".well")?.getAttribute("data-sheet") === "closed");
  await openSheet(page);
  const doneIds = await page.locator("[data-done-rows] [data-done-row]").evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute("data-approval-id")),
  );
  const waitingAfter = await rowIds(page);
  const check6Pass =
    beforeExpire.join(",") === "ap_exp,ap_keep" &&
    afterExpire.join(",") === beforeExpire.join(",") &&
    expiredTitle === "Expired · Reviewer will ask again on the next push." &&
    ring.includes("rgb") &&
    doneIds.join(",") === "ap_exp" &&
    waitingAfter.join(",") === "ap_keep";
  record(
    6,
    check6Pass ? "PASS" : "FAIL",
    `index held ${beforeExpire.join(" → ")} → ${afterExpire.join(" → ")}; "${expiredTitle}"; ring ${ring}; next open done ${doneIds.join(",")} waiting ${waitingAfter.join(",")}`,
  );

  mode = "hello";
  fixture = baseSnapshot(
    [
      approval({ id: "ap_h1", created_at: T_1114, expires_at: Date.now() + 15 * 60 * 1000 }),
      approval({ id: "ap_h2", created_at: T_1152, expires_at: Date.now() + 15 * 60 * 1000, action: "open_issue" }),
    ],
    [
      eventRow("ev_h1", "approval-requested:ap_h1", `${T_1114}:0:node`),
      eventRow("ev_h2", "approval-requested:ap_h2", `${T_1152}:0:node`),
    ],
  );
  await page.goto(origin, { waitUntil: "networkidle" });
  const helloCard = page.locator('article[data-approval-id="ap_h1"]');
  await helloCard.waitFor();
  await helloCard.focus();
  await page.locator(".hold-hint.armed").waitFor({ timeout: 5000 });
  await openSheet(page);
  const orderBeforeHello = await rowIds(page);
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.querySelector(".well")?.getAttribute("data-sheet") === "closed");
  await page.waitForFunction(
    () => document.querySelector('article[data-approval-id="ap_h1"]')?.getAttribute("data-seen") === "armed",
    null,
    { timeout: 5000 },
  );
  await page.getByRole("button", { name: "Approve draft" }).click();
  await page.getByText("Confirm with Windows Hello").waitFor({ timeout: 4000 });
  await shot(page, "check-07-hello");
  await page.getByRole("button", { name: "Cancel" }).click();
  await page.getByRole("button", { name: "Approve draft" }).waitFor();
  const afterCancel = await page.evaluate(() => {
    const card = document.querySelector('article[data-approval-id="ap_h1"]');
    return {
      hello: card?.getAttribute("data-hello") ?? "",
      seen: card?.getAttribute("data-seen") ?? "",
      alerts: document.querySelectorAll("[role=alert]").length,
      statusText: card?.innerText ?? "",
    };
  });
  await openSheet(page);
  const orderAfterHello = await rowIds(page);
  await shot(page, "check-07");
  const check7Pass =
    orderBeforeHello.join(",") === "ap_h1,ap_h2" &&
    orderAfterHello.join(",") === orderBeforeHello.join(",") &&
    afterCancel.hello === "off" &&
    afterCancel.alerts === 0 &&
    afterCancel.statusText.includes("Approve draft") &&
    !afterCancel.statusText.includes("Confirm with Windows Hello");
  record(
    7,
    check7Pass ? "PASS" : "FAIL",
    `rows ${orderBeforeHello.join(" → ")} then ${orderAfterHello.join(" → ")}; hello ${afterCancel.hello}; alerts ${afterCancel.alerts}`,
  );

  if (logs.length > 0) {
    console.log("page errors:");
    for (const line of logs) {
      console.log(line);
    }
  }
  const failed = checks.filter((check) => check.result !== "PASS");
  await writeFile(
    `${outDir}results.json`,
    JSON.stringify({ timezone: "America/New_York", viewport: "1280x800", deviceScaleFactor: 2, checks, pageErrors: logs }, null, 2),
  );
  if (failed.length > 0 || logs.length > 0) {
    throw new Error(`${failed.length} checks failed; ${logs.length} page errors`);
  }
  console.log("queue-proof: pass");
} finally {
  if (browser) {
    await browser.close();
  }
  preview.kill("SIGTERM");
}
