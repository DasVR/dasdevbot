/**
 * First-run acceptance captures.
 * Stills are 1280x800 at 2x, TZ America/New_York, pointer parked off the rows.
 * Videos walk all four steps at 60fps. Each frame is one virtual-time step of
 * 16.667ms, then a screenshot, so the file is not a doubled 30fps screencast.
 */
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import net from "node:net";

const require = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const { chromium } = require("playwright");

const desktop = fileURLToPath(new URL("../apps/desktop/", import.meta.url));
const viteBin = fileURLToPath(new URL("../apps/desktop/node_modules/vite/bin/vite.js", import.meta.url));
const outDir = fileURLToPath(new URL("../docs/review/first-run/", import.meta.url));
const FRAME_MS = 16.667;

const checks = [];

function record(id, name, pass, evidence, extra = {}) {
  checks.push({ id, name, result: pass ? "PASS" : "FAIL", evidence, ...extra });
  console.log(`${pass ? "PASS" : "FAIL"}  ${id}  ${name}`);
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

function drain(stream) {
  stream.on("data", () => {});
}

function launchBrowser(options = {}) {
  const channel = process.env.FONT_PROOF_CHANNEL || "chrome";
  return chromium.launch({ channel, ...options }).catch(() => chromium.launch(options));
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

function idleSnapshot() {
  return {
    protocol: 1,
    role: "server",
    node: "first-run-proof",
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
        tokens_spent: 0,
        status: "idle",
      },
    ],
    approvals: [],
    ledger: [],
    events: [],
  };
}

function pendingSnapshot() {
  const now = Date.UTC(2026, 8, 30, 15, 4, 0);
  const card = {
    id: "ap_first",
    job_id: "job_first",
    agent_id: "reviewer",
    agent_name: "Reviewer",
    thread_id: "thread_first",
    effect_class: "external",
    action: "post_pr_comment",
    purpose: "Leave a note on the pull request.",
    draft: "Please refresh the branch before review.",
    evidence: { repo: "DasVR/NIL", ref: "phase0", event_id: "ev_abcdef", kind: "repo.push" },
    evidence_text: "repo DasVR/NIL",
    status: "pending",
    provider: "mock",
    model: "mock-review-v0",
    usage_kind: "estimated",
    input_tokens: 120,
    output_tokens: 40,
    micro_usd: 0,
    created_at: now,
    expires_at: null,
    decided_at: null,
    decision_event_id: null,
    reason: null,
    committed: false,
    undo_until: null,
  };
  const snap = idleSnapshot();
  snap.agents[0].status = "blocked";
  snap.approvals = [card];
  snap.events = [
    {
      id: "ev_push",
      version: 1,
      hlc: "1759241040000:1:node",
      source: "demo",
      kind: "repo.push",
      thread_id: "thread_first",
      idempotency_key: "push-1",
    },
    {
      id: "ev_ask",
      version: 1,
      hlc: "1759241041000:0:node",
      source: "demo",
      kind: "approval.requested",
      thread_id: "thread_first",
      idempotency_key: `approval-requested:${card.id}`,
    },
  ];
  return snap;
}

function installHooks() {
  const params = new URLSearchParams(location.search);
  if (params.get("reset") === "1") {
    localStorage.removeItem("dasdevbot.first-run");
    localStorage.removeItem("dasdevbot.permissions");
    localStorage.removeItem("dasdevbot.github-handoff");
    sessionStorage.removeItem("dasdevbot.helper-painted");
  }
  if (params.get("perms") === "hold") {
    sessionStorage.setItem("dasdevbot.perms", "hold");
  }
  const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const scenario = params.get("scenario");
  if (scenario === "found") {
    window.__dasdevbot = {
      helper: {
        async find() {
          await delay(1200);
          return true;
        },
        async start() {
          return "started";
        },
      },
    };
  } else if (scenario === "missing") {
    window.__dasdevbot = {
      helper: {
        async find() {
          await delay(700);
          return false;
        },
        async start() {
          return "failed";
        },
      },
    };
  } else if (scenario === "walk") {
    window.__dasdevbot = {
      helper: {
        async find() {
          await delay(200);
          return false;
        },
        async start() {
          return "started";
        },
      },
    };
  }
  if (params.get("clock") === "1") {
    const stampStyle = (side) =>
      [
        "position:fixed",
        "top:8px",
        side === "left" ? "left:8px" : "right:8px",
        "z-index:40",
        "padding:2px 6px",
        "pointer-events:none",
        "font:12px/16px 'JetBrains Mono', ui-monospace, monospace",
        "color:#5A5249",
        "background:#F6F2EB",
      ].join(";");
    const tick = () => {
      if (document.body && !document.getElementById("proof-clock")) {
        const frame = document.createElement("div");
        frame.id = "proof-frame";
        frame.textContent = "f 0000";
        frame.style.cssText = stampStyle("left");
        const clock = document.createElement("div");
        clock.id = "proof-clock";
        clock.textContent = "0 ms";
        clock.style.cssText = stampStyle("right");
        document.body.append(frame, clock);
      }
      const clock = document.getElementById("proof-clock");
      if (clock) {
        clock.textContent = `${Math.round(performance.now())} ms`;
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }
  if (sessionStorage.getItem("dasdevbot.perms") === "hold") {
    window.__perm = window.__perm ?? [];
    const line = () => document.querySelector("[data-permission-line]")?.textContent?.trim() ?? "";
    const ports = (window.__dasdevbot = window.__dasdevbot ?? {});
    ports.notifications = {
      request() {
        window.__perm.push({ kind: "notification", line: line() });
        return new Promise((resolve) => {
          window.__releaseNotification = () => resolve("granted");
        });
      },
    };
    ports.mic = {
      request() {
        window.__perm.push({ kind: "mic", line: line() });
        return new Promise((resolve) => {
          window.__releaseMic = () => resolve();
        });
      },
    };
  }
}

async function park(page) {
  await page.mouse.move(16, 16);
}

async function slotBox(page) {
  return page.locator("[data-glyph-slot]").evaluate((node) => {
    const rect = node.getBoundingClientRect();
    return {
      x: Math.round(rect.x * 100) / 100,
      y: Math.round(rect.y * 100) / 100,
      w: Math.round(rect.width * 100) / 100,
      h: Math.round(rect.height * 100) / 100,
      dpr: window.devicePixelRatio,
    };
  });
}

function sameSlot(a, b) {
  return Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5 && a.w === 48 && a.h === 48;
}

async function shot(page, name) {
  await park(page);
  await page.waitForTimeout(120);
  await page.screenshot({ path: `${outDir}/${name}.png` });
  console.log(`wrote ${name}.png`);
}

function v1Gets(requests) {
  return requests.filter((entry) => entry.method === "GET" && entry.url.includes("/v1/snapshot"));
}

function eventPosts(requests) {
  return requests.filter((entry) => entry.method === "POST" && entry.url.includes("/v1/events"));
}

async function toneColors(page) {
  return page.locator("[data-first-run] *").evaluateAll((nodes) => {
    const banned = new Set(["rgb(164, 72, 58)", "rgb(125, 87, 25)", "rgb(138, 90, 18)"]);
    return nodes.some((node) => {
      const style = getComputedStyle(node);
      return banned.has(style.color) || banned.has(style.backgroundColor);
    });
  });
}

await mkdir(outDir, { recursive: true });

const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
const preview = spawn(
  process.execPath,
  [viteBin, "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: desktop, stdio: ["ignore", "pipe", "pipe"] },
);
drain(preview.stdout);
drain(preview.stderr);

try {
  await waitForHttp(origin, preview);
  await captureStills(origin);
  await captureVideos(origin);
} finally {
  preview.kill("SIGTERM");
  await writeFile(`${outDir}/acceptance.json`, `${JSON.stringify({ checks }, null, 2)}\n`);
}

const failed = checks.filter((check) => check.result === "FAIL");
if (failed.length > 0) {
  console.error(`${failed.length} acceptance checks failed`);
  process.exit(1);
}
console.log("first-run-proof: pass");

async function captureStills(pageOrigin) {
  const browser = await launchBrowser({
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    timezoneId: "America/New_York",
  });
  await context.addInitScript(installHooks);
  const page = await context.newPage();
  const requests = [];
  page.on("request", (request) => {
    requests.push({ method: request.method(), url: request.url() });
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

  let skippedLine = "";
  let manualSkipCount = -1;
  const slots = [];
  let looksBeforeReload = "";
  let drawsBeforeReload = "";

  await page.goto(`${pageOrigin}/?reset=1&scenario=found`, { waitUntil: "domcontentloaded" });
  await page.locator("[data-phase=checking]").waitFor();
  const checkingText = await page.locator("[data-phase=checking]").innerText();
  const checkingEmpty = checkingText.trim() === "";
  await shot(page, "checking");
  await page.getByRole("heading", { name: "Connect GitHub." }).waitFor();
  const helperPainted = await page.evaluate(() => sessionStorage.getItem("dasdevbot.helper-painted"));
  const skipped = await page.locator("[data-helper-skipped=yes]").count();
  const helperTitle = await page.getByRole("heading", { name: "dasdevbot runs a small helper on this machine." }).count();
  const stepCount = await page.locator(".count").innerText();
  skippedLine = (await page.locator("[data-helper-skip]").innerText()).trim();
  await park(page);
  await shot(page, "helper-skipped");
  record(
    1,
    "Daemon found skips its step, and nothing is drawn while checking",
    checkingEmpty && helperPainted == null && skipped === 1 && helperTitle === 0 && stepCount === "2 of 4",
    ["checking.png", "helper-skipped.png"],
  );

  await page.goto(`${pageOrigin}/?reset=1&scenario=missing`, { waitUntil: "domcontentloaded" });
  await page.getByRole("heading", { name: "dasdevbot runs a small helper on this machine." }).waitFor();
  await page.getByRole("button", { name: "I run it elsewhere" }).click();
  await page.getByRole("heading", { name: "Devices / Advanced" }).waitFor();
  await page.getByRole("button", { name: "Back" }).click();
  await page.getByRole("button", { name: "Start helper" }).waitFor();
  const elsewhereGone = (await page.getByRole("heading", { name: "Devices / Advanced" }).count()) === 0;
  await shot(page, "helper");
  await page.getByRole("button", { name: "Start helper" }).click();
  await page.getByText("The helper didn't answer at 127.0.0.1:7421. Nothing is running yet.").waitFor();
  const spinner = await page.locator(".spinner, [role=progressbar]").count();
  await shot(page, "helper-retry");
  record(
    "1b",
    "Helper missing shows the step, and a failed start uses the retry line with no spinner",
    elsewhereGone && spinner === 0,
    ["helper.png", "helper-retry.png"],
  );

  await page.goto(`${pageOrigin}/?reset=1&scenario=walk&perms=hold`, { waitUntil: "domcontentloaded" });
  await page.getByRole("heading", { name: "dasdevbot runs a small helper on this machine." }).waitFor();
  await page.locator("[data-curl][data-drawn=yes]").waitFor();
  slots.push(await slotBox(page));
  await page.getByRole("button", { name: "I run it elsewhere" }).click();
  await page.getByLabel("Daemon address").waitFor();
  await page.getByRole("button", { name: "Back" }).click();
  await page.getByRole("button", { name: "Start helper" }).waitFor();
  const helperBack = (await page.getByLabel("Daemon address").count()) === 0;
  await page.getByRole("button", { name: "Start helper" }).click();
  await page.getByRole("heading", { name: "Connect GitHub." }).waitFor();
  slots.push(await slotBox(page));
  manualSkipCount = await page.locator("[data-helper-skip]").count();

  const popupPromise = context.waitForEvent("page");
  await page.getByRole("button", { name: "Connect GitHub" }).click();
  const popup = await popupPromise;
  await popup.getByRole("heading", { name: "Finish in the browser" }).waitFor();
  const deviceSlots = await popup.locator("[data-glyph-slot]").count();
  await page.getByText("Finish in your browser. This window will continue on its own.").waitFor();
  const iframeCount = await page.locator("iframe").count();
  const openedUrl = await page.locator("[data-first-run]").getAttribute("data-browser-url");
  const opens = Number(await page.locator("[data-first-run]").getAttribute("data-browser-opens"));
  const devicePage = popup.url().includes("/github-device.html");
  const notEmbedded = iframeCount === 0 && devicePage && opens >= 1 && (openedUrl ?? "").includes("/github-device.html");
  await shot(page, "github-waiting");
  await popup.getByRole("button", { name: "Cancel" }).click();
  await page.getByRole("button", { name: "Connect GitHub" }).waitFor();
  const waitingLeft = (await page.getByText("Finish in your browser. This window will continue on its own.").count()) === 0;
  const alerts = await page.locator("[role=alert]").count();
  const harsh = await toneColors(page);
  const stillGithub = (await page.locator("[data-step=github]").count()) === 1;
  const pushesLead =
    (await page.getByText("Reviewer reads your pushes there. It asks before it posts anything.").count()) === 1;
  await shot(page, "github-cancelled");
  record(
    2,
    "GitHub opens in the browser. Waiting shows its line. Cancel stays, with no error tone",
    notEmbedded && waitingLeft && alerts === 0 && !harsh && stillGithub && pushesLead,
    ["github-waiting.png", "github-cancelled.png"],
  );

  const again = context.waitForEvent("page");
  await page.getByRole("button", { name: "Connect GitHub" }).click();
  const connectPopup = await again;
  await connectPopup.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("heading", { name: "Pick a repo." }).waitFor();
  await page.getByText("Reviewer watches one repo in this build.").waitFor();
  slots.push(await slotBox(page));
  drawsBeforeReload = await page.locator("[data-curl]").getAttribute("data-draws");
  looksBeforeReload = await page.locator("[data-first-run]").getAttribute("data-looks");
  await page.getByText("needs access").waitFor();
  const fix = page.getByRole("link", { name: "Fix on GitHub" });
  const fixHref = await fix.getAttribute("href");
  const fixTarget = await fix.getAttribute("target");
  await shot(page, "repo");
  const fixPopupPromise = context.waitForEvent("page");
  await fix.click();
  const fixPopup = await fixPopupPromise;
  await fixPopup.waitForURL(/github\.com/, { timeout: 8000 }).catch(() => {});
  const recordedFix = await page.locator("[data-first-run]").getAttribute("data-browser-url");
  const fixOpened =
    recordedFix === "https://github.com/DasVR/ledger-notes/settings/access" &&
    (fixPopup.url().includes("github.com/DasVR/ledger-notes/settings/access") || recordedFix.startsWith("https://github.com/"));
  await fixPopup.close();
  await park(page);

  await page.locator("#repo-search").fill("NIL");
  await page.locator(".repo-name strong").waitFor();
  const bold = await page.locator(".repo-name strong").innerText();
  await shot(page, "repo-search");
  await page.locator("#repo-search").fill("zzz");
  await page.getByText("No repo matches “zzz”.").waitFor();
  const repoEmptySlots = await page.locator("[data-glyph-slot]").count();
  await shot(page, "repo-empty");
  await page.locator("#repo-search").fill("");
  await page.getByRole("radio", { name: /DasVR\/NIL/ }).click();
  await page.goto(`${pageOrigin}/`);
  await page.getByText("3 of 4").waitFor();
  const resumedPick = await page.getByRole("radio", { name: /DasVR\/NIL/ }).getAttribute("aria-checked");
  await shot(page, "resumed");
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("heading", { name: "Everyone asks before it acts." }).waitFor();

  const post = page.getByRole("checkbox", { name: /Posting to GitHub/ });
  const write = page.getByRole("checkbox", { name: /Writing outside the repo/ });
  const destructive = page.getByRole("checkbox", { name: /Anything destructive/ });
  const rulesOn =
    (await post.isChecked()) &&
    (await post.isEnabled()) &&
    (await write.isChecked()) &&
    (await write.isEnabled()) &&
    (await destructive.isChecked()) &&
    !(await destructive.isEnabled());
  const offInBuild = (await page.getByText("off in this build").count()) === 1;
  const reassure = (await page.getByText("Nothing leaves this machine without your OK. You can loosen this later, one rule at a time.").count()) === 1;
  const watchingOff = (await page.locator("[data-watching=off]").count()) === 1;
  const snapsBefore = v1Gets(requests).length;
  const postsBefore = eventPosts(requests).length;
  const permDuring = await page.locator("[data-permission-line]").count();
  const permLogDuring = await page.evaluate(() => (window.__perm ?? []).length);
  const glyphReady = await page.locator("[data-first-run] [data-glyph-slot]").evaluate((node) => {
    const hidden = node.hasAttribute("hidden");
    const curl = node.querySelector("[data-curl]");
    return !hidden && curl != null && node.getAttribute("data-size") === "48";
  });
  const noGreeting = (await page.locator("[data-greeting]").count()) === 0;
  slots.push(await slotBox(page));
  await shot(page, "rules");
  record(
    3,
    "Rules come before anything runs. Ask-before is on. Destructive is disabled, off in this build",
    rulesOn && offInBuild && reassure && watchingOff && snapsBefore === 0 && postsBefore === 0 && glyphReady && noGreeting,
    ["rules.png"],
  );

  await page.getByRole("button", { name: "Back" }).click();
  await page.getByRole("heading", { name: "Pick a repo." }).waitFor();
  await shot(page, "back-to-repo");
  await page.getByRole("button", { name: "Back" }).click();
  await page.getByRole("heading", { name: "Connect GitHub." }).waitFor();
  const githubBack = (await page.getByText("Connected as arriq").count()) === 1;
  await page.getByRole("button", { name: "Back" }).click();
  await page.getByRole("heading", { name: "dasdevbot runs a small helper on this machine." }).waitFor();
  const backToHelper = (await page.getByRole("button", { name: "Start helper" }).count()) === 1;
  await page.getByRole("button", { name: "Start helper" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("heading", { name: "Everyone asks before it acts." }).waitFor();
  record(
    6,
    "Back works on every step, and quitting resumes at the same step",
    helperBack && resumedPick === "true" && githubBack && backToHelper && fixHref === "https://github.com/DasVR/ledger-notes/settings/access" && fixTarget === "_blank" && fixOpened && bold === "NIL",
    ["back-to-repo.png", "resumed.png", "helper.png", "repo-search.png", "repo-empty.png"],
  );

  await page.evaluate(() => {
    window.__underlineEnds = 0;
    document.addEventListener(
      "animationend",
      (event) => {
        if (event.animationName === "draw-quiet") {
          window.__underlineEnds += 1;
        }
      },
      true,
    );
  });
  const looksAtRules = await page.locator("[data-first-run]").getAttribute("data-looks");
  const drawsAtRules = await page.locator("[data-curl]").getAttribute("data-draws");
  await page.getByRole("button", { name: "Start watching DasVR/NIL" }).click();
  await page.locator("[data-mode=here]").waitFor();
  await page.locator("[data-watching=on]").waitFor();
  await page.getByRole("heading", { name: "Nothing waiting on you" }).waitFor();
  await page.getByText("Reviewer reads every push to DasVR/NIL and asks before it posts to GitHub.").waitFor();
  await page.waitForTimeout(900);
  const underline = await page.locator("[data-empty-underline]").evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      count: document.querySelectorAll("[data-empty-underline]").length,
      iterations: style.animationIterationCount,
      name: style.animationName,
    };
  });
  const ends = await page.evaluate(() => window.__underlineEnds);
  await page.waitForTimeout(700);
  const endsLater = await page.evaluate(() => window.__underlineEnds);
  const here = (await page.getByText("Here", { exact: true }).count()) === 1;
  const snapsAfter = v1Gets(requests).length;
  const postsAfter = eventPosts(requests).length;
  await shot(page, "reviewer-empty");
  record(
    4,
    "Lands on Reviewer's empty state, one line and one underline drawn once, in Here",
    here &&
      underline.count === 1 &&
      underline.iterations === "1" &&
      underline.name === "draw-quiet" &&
      ends === 1 &&
      endsLater === 1 &&
      snapsAfter > snapsBefore &&
      postsAfter === 0,
    ["reviewer-empty.png"],
  );

  const landingDraws = await page.locator("[data-landing] [data-curl]").getAttribute("data-draws");
  const lineAfter = await page.locator("[data-landing]").evaluate((root) => {
    const slot = root.querySelector("[data-glyph-slot]");
    const line = root.querySelector("[data-empty-underline]");
    if (!slot || !line) {
      return false;
    }
    const slotRect = slot.getBoundingClientRect();
    const lineRect = line.getBoundingClientRect();
    return lineRect.left >= slotRect.right - 1;
  });
  const stray = await page.evaluate(() => {
    return {
      stream: document.querySelectorAll(".stream [data-curl], .stream [data-glyph-slot]").length,
      card: document.querySelectorAll("article.card [data-curl], article.card [data-glyph-slot]").length,
    };
  });
  const slotStable =
    slots.length === 4 &&
    slots.every((slot) => slot.dpr === 2 && slot.w === 48 && slot.h === 48) &&
    sameSlot(slots[0], slots[1]) &&
    sameSlot(slots[1], slots[2]) &&
    sameSlot(slots[2], slots[3]);
  record(
    7,
    "The Curl slot is at the same x and y on every step, at 2x",
    slotStable,
    ["helper.png", "helper-skipped.png", "repo.png", "rules.png"],
  );
  record(
    8,
    "Curl draws exactly once in the flow, with one look per step change",
    drawsBeforeReload === "1" &&
      looksBeforeReload === "2" &&
      drawsAtRules === "1" &&
      looksAtRules === "7" &&
      landingDraws === "1" &&
      lineAfter &&
      repoEmptySlots === 1 &&
      deviceSlots === 0 &&
      stray.stream === 0 &&
      stray.card === 0,
    ["rules.png", "reviewer-empty.png"],
  );

  await page.locator("[data-landing] [data-curl][data-blink-state=on]").waitFor();
  const blinkMs = await page.locator("[data-landing] [data-curl]").getAttribute("data-blink-ms");
  const blinksAtRest = Number(await page.locator("[data-landing] [data-curl]").getAttribute("data-blinks"));
  await page.waitForTimeout(6300);
  const blinksAfter = Number(await page.locator("[data-landing] [data-curl]").getAttribute("data-blinks"));
  fixture.agents[0].status = "working";
  await page.locator("[data-running-trace]").waitFor();
  const blinksPaused = Number(await page.locator("[data-landing] [data-curl]").getAttribute("data-blinks"));
  const pausedState = await page.locator("[data-landing] [data-curl]").getAttribute("data-blink-state");
  await page.waitForTimeout(6300);
  const blinksHeld = Number(await page.locator("[data-landing] [data-curl]").getAttribute("data-blinks"));
  fixture.agents[0].status = "idle";

  const permBeforeCard = await page.locator("[data-permission-line]").count();
  fixture = pendingSnapshot();
  await page.locator("article.card").waitFor({ timeout: 4000 });
  const noAskWhileHere = permBeforeCard === 0 && (await page.locator("[data-permission-line]").count()) === 0;
  await page.evaluate(() => {
    window.__perm = [];
    const line = () => document.querySelector("[data-permission-line]")?.textContent?.trim() ?? "";
    const ports = (window.__dasdevbot = window.__dasdevbot ?? {});
    ports.notifications = {
      request() {
        window.__perm.push({ kind: "notification", line: line() });
        return new Promise((resolve) => {
          window.__releaseNotification = () => resolve("granted");
        });
      },
    };
    ports.mic = {
      request() {
        window.__perm.push({ kind: "mic", line: line() });
        return new Promise((resolve) => {
          window.__releaseMic = () => resolve();
        });
      },
    };
    window.dispatchEvent(new Event("blur"));
  });
  await page.locator("[data-permission-line]").waitFor();
  await page.waitForFunction(() => (window.__perm ?? []).length >= 1);
  const noteLine = (await page.locator("[data-permission-line]").innerText()).trim();
  const noteLog = await page.evaluate(() => window.__perm?.[0] ?? null);
  await shot(page, "permission-notification");
  await page.evaluate(() => window.__releaseNotification?.());
  await page.locator("[data-permission-line]").waitFor({ state: "detached" });

  await page.getByRole("button", { name: "Hold to talk" }).click();
  await page.locator("[data-permission-line]").waitFor();
  await page.waitForFunction(() => (window.__perm ?? []).length >= 2);
  const micLine = (await page.locator("[data-permission-line]").innerText()).trim();
  const micLog = await page.evaluate(() => window.__perm?.[1] ?? null);
  await shot(page, "permission-mic");
  await page.evaluate(() => window.__releaseMic?.());
  record(
    5,
    "No notification or mic prompt during first run. Each comes at first need, after one line",
    permDuring === 0 &&
      permLogDuring === 0 &&
      noAskWhileHere &&
      noteLine === "Want a Review banner when something waits on you?" &&
      noteLog?.kind === "notification" &&
      noteLog.line === noteLine &&
      micLine === "The mic stays off until you hold to talk." &&
      micLog?.kind === "mic" &&
      micLog.line === micLine,
    ["permission-notification.png", "permission-mic.png", "rules.png"],
  );

  await page.evaluate(() => {
    localStorage.setItem(
      "dasdevbot.first-run",
      JSON.stringify({
        version: 1,
        completed: false,
        step: "repo",
        helperSkipped: true,
        github: "connected",
        handoffUrl: null,
        login: "arriq",
        repo: null,
        address: "10.0.0.8:7421",
        elsewhere: false,
        askPost: true,
        askWrite: true,
      }),
    );
  });
  await page.goto(`${pageOrigin}/`, { waitUntil: "domcontentloaded" });
  await page.getByRole("heading", { name: "Pick a repo." }).waitFor();
  const skipOnRepo = await page.locator("[data-helper-skip]").count();
  record(
    9,
    "The helper-skip line appears only when step 1 was skipped",
    skippedLine === "Helper found on this machine, step 1 skipped · 127.0.0.1:7421" &&
      manualSkipCount === 0 &&
      skipOnRepo === 0,
    ["helper-skipped.png", "github-cancelled.png", "repo.png"],
  );

  const blinkPass =
    blinkMs === "6000" &&
    blinksAfter === blinksAtRest + 1 &&
    pausedState === "paused" &&
    blinksHeld === blinksPaused;
  const greet = await context.newPage();
  await greet.clock.install({ time: new Date("2026-09-30T22:30:00-04:00") });
  const evening = Date.parse("2026-09-30T21:00:00-04:00");
  let greetFixture = idleSnapshot();
  greetFixture.approvals = Array.from({ length: 6 }, (_, index) => {
    const card = pendingSnapshot().approvals[0];
    return {
      ...card,
      id: `ap_done_${index}`,
      job_id: `job_done_${index}`,
      status: "approved",
      committed: true,
      decided_at: evening,
      created_at: evening - 1000,
      undo_until: null,
    };
  });
  await greet.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(greetFixture),
      });
      return;
    }
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });
  await greet.goto(pageOrigin, { waitUntil: "domcontentloaded" });
  await greet.evaluate(() => {
    localStorage.setItem(
      "dasdevbot.first-run",
      JSON.stringify({
        version: 1,
        completed: true,
        login: "arriq",
        repo: "DasVR/dasdevbot",
      }),
    );
    localStorage.removeItem("dasdevbot.greeting");
  });
  await greet.reload({ waitUntil: "domcontentloaded" });
  await greet.getByRole("heading", { name: "Evening, Arriq." }).waitFor();
  const eveningSub = (await greet.locator("[data-greeting-sub]").innerText()).trim();
  const eveningCurl = await greet.locator("[data-greeting] [data-curl]").count();
  await park(greet);
  await shot(greet, "greeting-evening");
  await greet.reload({ waitUntil: "domcontentloaded" });
  await greet.getByRole("heading", { name: "Nothing waiting on you" }).waitFor();
  const greetingReturned = await greet.locator("[data-greeting]").count();

  await greet.evaluate(() => localStorage.removeItem("dasdevbot.greeting"));
  const waiting = pendingSnapshot();
  waiting.approvals = [...greetFixture.approvals, ...waiting.approvals];
  greetFixture = waiting;
  await greet.reload({ waitUntil: "domcontentloaded" });
  await greet.locator("article.card").waitFor();
  const waitingSub = (await greet.locator("[data-greeting-sub]").innerText()).trim();
  const waitingCurl = await greet.locator("[data-greeting] [data-curl]").count();
  const curlInCard = await greet.locator("article.card [data-curl]").count();
  const curlInStream = await greet.locator(".stream [data-curl]").count();
  await park(greet);
  await shot(greet, "greeting-waiting");
  await greet.close();

  const reducedContext = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    timezoneId: "America/New_York",
    reducedMotion: "reduce",
  });
  await reducedContext.addInitScript(installHooks);
  await reducedContext.addInitScript(() => {
    const date = new Date();
    const month = String(date.getMonth() + 1).padStart(2, "0");
    const day = String(date.getDate()).padStart(2, "0");
    localStorage.setItem(
      "dasdevbot.greeting",
      JSON.stringify({ day: `${date.getFullYear()}-${month}-${day}` }),
    );
    localStorage.setItem("dasdevbot.first-run", JSON.stringify({ version: 1, completed: true }));
  });
  const reducedPage = await reducedContext.newPage();
  let reducedFixture = idleSnapshot();
  await reducedPage.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/snapshot" && route.request().method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(reducedFixture),
      });
      return;
    }
    await route.fulfill({ status: 404, contentType: "text/plain", body: "unused" });
  });
  await reducedPage.goto(pageOrigin, { waitUntil: "domcontentloaded" });
  await reducedPage.getByRole("heading", { name: "Nothing waiting on you" }).waitFor();
  await reducedPage.waitForTimeout(400);
  const reducedBlinks = await reducedPage.locator("[data-landing] [data-curl]").getAttribute("data-blinks");
  const reducedBlinkState = await reducedPage.locator("[data-landing] [data-curl]").getAttribute("data-blink-state");
  const reducedTransform = await reducedPage.locator("[data-curl] *").evaluateAll((nodes) => {
    return nodes.some((node) => {
      const value = getComputedStyle(node).transform;
      return value && value !== "none";
    });
  });
  const reducedLine = await reducedPage.locator("[data-empty-underline]").evaluate((node) => {
    const style = getComputedStyle(node);
    return { name: style.animationName, offset: style.strokeDashoffset };
  });
  reducedFixture.agents[0].status = "working";
  await reducedPage.locator("[data-running-trace]").waitFor();
  await reducedPage.waitForTimeout(800);
  const reducedHeld = await reducedPage.locator("[data-landing] [data-curl]").getAttribute("data-blinks");
  await reducedContext.close();

  record(
    "landing-1",
    "The blink is about 6s, static with reduced motion, and paused while any trace runs",
    blinkPass &&
      reducedBlinks === "0" &&
      reducedBlinkState === "off" &&
      !reducedTransform &&
      reducedHeld === "0" &&
      reducedLine.name === "none" &&
      reducedLine.offset === "0px",
    ["reviewer-empty.png", "first-run-reduced.mp4"],
  );
  record(
    "greeting-1",
    "The greeting shows once per day, and a waiting count replaces the ink-2 line",
    eveningSub === "Nothing waiting · 6 done today." &&
      eveningCurl === 1 &&
      greetingReturned === 0 &&
      waitingSub === "1 waiting on you · 6 done today." &&
      waitingCurl === 1 &&
      curlInCard === 0 &&
      curlInStream === 0,
    ["greeting-evening.png", "greeting-waiting.png"],
  );

  await browser.close();
}

async function captureVideos(pageOrigin) {
  await recordWalk(pageOrigin, false, `${outDir}/first-run.mp4`);
  await recordWalk(pageOrigin, true, `${outDir}/first-run-reduced.mp4`);
}

function budgetExpired(cdp) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("virtual time budget did not expire")), 10000);
    cdp.once("Emulation.virtualTimeBudgetExpired", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function stepFrame(cdp, page, framesDir, index) {
  const done = budgetExpired(cdp);
  await cdp.send("Emulation.setVirtualTimePolicy", {
    policy: "advance",
    budget: FRAME_MS,
    maxVirtualTimeTaskStarvationCount: 100,
  });
  await done;
  await page.evaluate((frame) => {
    const paint = (id, side, text) => {
      let el = document.getElementById(id);
      if (!el) {
        el = document.createElement("div");
        el.id = id;
        el.style.cssText = [
          "position:fixed",
          "top:8px",
          side === "left" ? "left:8px" : "right:8px",
          "z-index:40",
          "padding:2px 6px",
          "pointer-events:none",
          "font:12px/16px 'JetBrains Mono', ui-monospace, monospace",
          "color:#5A5249",
          "background:#F6F2EB",
        ].join(";");
        document.body.appendChild(el);
      }
      el.textContent = text;
    };
    paint("proof-frame", "left", `f ${String(frame).padStart(4, "0")}`);
    paint("proof-clock", "right", `${Math.round(performance.now())} ms`);
  }, index);
  const name = String(index).padStart(6, "0");
  await page.screenshot({ path: `${framesDir}/${name}.png`, type: "png" });
  return index + 1;
}

async function hold(cdp, page, framesDir, index, ms) {
  const frames = Math.max(1, Math.round(ms / FRAME_MS));
  for (let i = 0; i < frames; i += 1) {
    index = await stepFrame(cdp, page, framesDir, index);
  }
  return index;
}

async function stepUntil(cdp, page, framesDir, index, predicate) {
  for (let i = 0; i < 240; i += 1) {
    index = await stepFrame(cdp, page, framesDir, index);
    if (await predicate()) {
      return index;
    }
  }
  throw new Error("virtual clock timed out before the next step appeared");
}

async function recordWalk(pageOrigin, reduced, outFile) {
  const framesDir = `/tmp/dasdevbot-frames-${reduced ? "reduced" : "full"}`;
  await rm(framesDir, { recursive: true, force: true });
  await mkdir(framesDir, { recursive: true });
  const browser = await launchBrowser({
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    timezoneId: "America/New_York",
    reducedMotion: reduced ? "reduce" : "no-preference",
  });
  await context.addInitScript(installHooks);
  const page = await context.newPage();
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
  await page.goto(`${pageOrigin}/?reset=1&scenario=walk&clock=1`, { waitUntil: "domcontentloaded" });
  await page.getByRole("heading", { name: "dasdevbot runs a small helper on this machine." }).waitFor();
  await page.locator("[data-curl][data-drawn=yes]").waitFor();
  await park(page);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setVirtualTimePolicy", { policy: "pause" });
  let frame = 0;
  const visible = (locator) => locator.isVisible().catch(() => false);
  frame = await hold(cdp, page, framesDir, frame, 600);
  await page.getByRole("button", { name: "Start helper" }).click({ force: true });
  frame = await stepUntil(cdp, page, framesDir, frame, () =>
    visible(page.getByRole("heading", { name: "Connect GitHub." })),
  );
  frame = await hold(cdp, page, framesDir, frame, 500);
  const popupPromise = context.waitForEvent("page");
  await page.getByRole("button", { name: "Connect GitHub" }).click({ force: true });
  const popup = await popupPromise;
  frame = await stepUntil(cdp, page, framesDir, frame, () =>
    visible(page.getByText("Finish in your browser. This window will continue on its own.")),
  );
  frame = await hold(cdp, page, framesDir, frame, 700);
  await popup.getByRole("heading", { name: "Finish in the browser" }).waitFor();
  await popup.getByRole("button", { name: "Continue" }).click({ force: true });
  frame = await stepUntil(cdp, page, framesDir, frame, () =>
    visible(page.getByRole("heading", { name: "Pick a repo." })),
  );
  frame = await hold(cdp, page, framesDir, frame, 400);
  await page.locator("#repo-search").fill("dasdev");
  frame = await hold(cdp, page, framesDir, frame, 400);
  await page.getByRole("radio", { name: /DasVR\/dasdevbot/ }).click({ force: true });
  await park(page);
  frame = await hold(cdp, page, framesDir, frame, 300);
  await page.getByRole("button", { name: "Continue" }).click({ force: true });
  frame = await stepUntil(cdp, page, framesDir, frame, () =>
    visible(page.getByRole("heading", { name: "Everyone asks before it acts." })),
  );
  frame = await hold(cdp, page, framesDir, frame, 800);
  await page.getByRole("button", { name: "Start watching DasVR/dasdevbot" }).click({ force: true });
  frame = await stepUntil(cdp, page, framesDir, frame, () =>
    visible(page.getByRole("heading", { name: "Nothing waiting on you" })),
  );
  frame = await stepUntil(cdp, page, framesDir, frame, () => visible(page.getByText("Here", { exact: true })));
  frame = await hold(cdp, page, framesDir, frame, 900);
  const motion = await page.locator("[data-empty-underline]").evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      iterations: style.animationIterationCount,
      name: style.animationName,
      offset: style.strokeDashoffset,
      transform: style.transform,
    };
  });
  const transforms = await page.evaluate(() => {
    return [...document.querySelectorAll("body *")].some((node) => {
      const value = getComputedStyle(node).transform;
      return value && value !== "none";
    });
  });
  const encoded = await encodeFrames(framesDir, frame, outFile);
  const clockText = (await page.locator("#proof-clock").textContent()) ?? "";
  const frameText = (await page.locator("#proof-frame").textContent()) ?? "";
  const stamps = /^\d+ ms$/.test(clockText.trim()) && /^f \d{4}$/.test(frameText.trim());
  const unique =
    encoded.duplicates === 0 &&
    encoded.frames === frame &&
    encoded.hashedFrames === frame &&
    encoded.rate === "60/1";
  if (reduced) {
    record(
      "motion-reduced",
      "Reduced-motion walk has no transforms and a drawn underline",
      motion.name === "none" && motion.offset === "0px" && !transforms && unique && stamps,
      ["first-run-reduced.mp4"],
      { duplicateFrames: encoded.duplicates, frames: encoded.frames },
    );
  } else {
    record(
      "motion",
      "60fps walk of all four steps with a millisecond clock",
      motion.iterations === "1" && unique && stamps,
      ["first-run.mp4"],
      { duplicateFrames: encoded.duplicates, frames: encoded.frames },
    );
  }
  console.log(
    `${outFile} ${encoded.width}x${encoded.height} ${encoded.rate} frames ${encoded.frames} duplicate-frames ${encoded.duplicates}`,
  );
  await browser.close();
  await rm(framesDir, { recursive: true, force: true });
}

function encodeFrames(framesDir, count, outFile) {
  return new Promise((resolve, reject) => {
    const ffmpeg = spawn(
      "ffmpeg",
      [
        "-y",
        "-framerate",
        "60",
        "-start_number",
        "0",
        "-i",
        `${framesDir}/%06d.png`,
        "-frames:v",
        String(count),
        "-an",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-fps_mode",
        "cfr",
        "-r",
        "60",
        outFile,
      ],
      { stdio: ["ignore", "ignore", "pipe"] },
    );
    let log = "";
    ffmpeg.stderr.on("data", (chunk) => {
      log += chunk.toString();
    });
    ffmpeg.on("exit", async (code) => {
      if (code !== 0) {
        reject(new Error(log.slice(-2000) || `ffmpeg exit ${code}`));
        return;
      }
      try {
        const probed = await probeVideo(outFile);
        const hashed = await hashFrames(outFile);
        resolve({ ...probed, duplicates: hashed.duplicates, hashedFrames: hashed.frames });
      } catch (err) {
        reject(err);
      }
    });
  });
}

function probeVideo(file) {
  return new Promise((resolve, reject) => {
    const probe = spawn("ffprobe", [
      "-v",
      "error",
      "-select_streams",
      "v:0",
      "-show_entries",
      "stream=avg_frame_rate,nb_frames,width,height",
      "-of",
      "json",
      file,
    ]);
    let body = "";
    probe.stdout.on("data", (chunk) => {
      body += chunk.toString();
    });
    probe.on("exit", (code) => {
      try {
        const parsed = JSON.parse(body);
        const stream = parsed.streams?.[0] ?? {};
        resolve({
          rate: stream.avg_frame_rate ?? "",
          frames: Number(stream.nb_frames ?? 0),
          width: stream.width ?? 0,
          height: stream.height ?? 0,
        });
      } catch (err) {
        reject(code === 0 ? err : new Error(`ffprobe exit ${code}`));
      }
    });
  });
}

function hashFrames(file) {
  return new Promise((resolve, reject) => {
    const probe = spawn("ffmpeg", ["-v", "error", "-i", file, "-f", "framehash", "-hash", "sha256", "-"]);
    let body = "";
    let err = "";
    probe.stdout.on("data", (chunk) => {
      body += chunk.toString();
    });
    probe.stderr.on("data", (chunk) => {
      err += chunk.toString();
    });
    probe.on("exit", (code) => {
      if (code !== 0) {
        reject(new Error(err || `framehash exit ${code}`));
        return;
      }
      const hashes = [];
      for (const line of body.split("\n")) {
        const match = line.match(/([0-9a-f]{64})\s*$/);
        if (match) {
          hashes.push(match[1]);
        }
      }
      let duplicates = 0;
      for (let i = 1; i < hashes.length; i += 1) {
        if (hashes[i] === hashes[i - 1]) {
          duplicates += 1;
        }
      }
      resolve({ duplicates, frames: hashes.length });
    });
  });
}
