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
 * Main window (label "main"): Alt+Down focuses the waiting step (UID 5);
 * no Approve control exists in any form, the
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

// The real daemon shape (store.rs record_gate_denial): destructive work is
// recorded denied and committed at once, and the only stream event is
// `gate-denied:<id>`. There is never an `approval-requested:<id>` or a pending
// destructive row.
const destructive = {
  ...card,
  id: "ap_force",
  job_id: "job_force",
  thread_id: "thread_force",
  effect_class: "destructive",
  action: "force_push",
  // The mock demo routes the scripted force-push to Builder (CD ruling c).
  agent_id: "builder",
  agent_name: "Builder",
  purpose: "Force-push phase0. This rewrites the remote branch.",
  draft: "git push --force origin phase0",
  evidence: { repo: "DasVR/NIL", ref: "phase0", event_id: "ev_force1", kind: "repo.force_push" },
  provider: "none",
  model: "",
  usage_kind: "none",
  input_tokens: 0,
  output_tokens: 0,
  status: "denied",
  decided_at: now,
  committed: true,
  undo_until: null,
};

const forceEvents = [
  {
    id: "ev_force2",
    version: 1,
    hlc: `${now + 1}:0:card-smoke`,
    source: "runtime",
    kind: "gate.denied",
    thread_id: "thread_force",
    idempotency_key: "gate-denied:ap_force",
  },
  {
    id: "ev_force1",
    version: 1,
    hlc: `${now}:0:card-smoke`,
    source: "demo",
    kind: "repo.force_push",
    thread_id: "thread_force",
    idempotency_key: "ui-force",
  },
];

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

function stub({ label, snapshot, helloMs = 0, cancel = false, decides = false }) {
  return [
    ({ label, snapshot, helloMs, cancel, decides }) => {
      window.__invokes = [];
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label } },
        setSnapshot(next) {
          snapshot = next;
        },
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
              setTimeout(() => {
                if (cancel) {
                  reject(new Error("Windows Hello consent was denied"));
                  return;
                }
                if (decides) {
                  // The daemon's answer from here on: decided, undo window open.
                  // Each signed decision is a fresh event (store.rs: Uuid::new_v4).
                  const at = Date.now();
                  window.__signed = (window.__signed ?? 0) + 1;
                  const status = args.decision === "deny" ? "denied" : "approved";
                  snapshot = {
                    ...snapshot,
                    approvals: snapshot.approvals.map((approval) => ({
                      ...approval,
                      status,
                      decided_at: at,
                      decision_event_id: `ev_${String(window.__signed).repeat(8)}decided`,
                      undo_until: at + 6000,
                    })),
                  };
                }
                resolve(null);
              }, helloMs),
            );
          }
          if (command === "undo_decision" && decides) {
            // Inside the window the daemon reverts to pending and clears the
            // decision event (store.rs undo_inside_the_window_reverts_to_pending).
            snapshot = {
              ...snapshot,
              approvals: snapshot.approvals.map((approval) => ({
                ...approval,
                status: "pending",
                decided_at: null,
                decision_event_id: null,
                undo_until: null,
              })),
            };
          }
          return Promise.resolve(null);
        },
      };
    },
    { label, snapshot, helloMs, cancel, decides },
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

  // UX 3: opened with a pending card, the card element itself has focus.
  // UX 4: the 800ms seen-lock dwell starts when the 520ms rise lands.
  {
    const timed = await browser.newPage({ viewport: { width: 560, height: 760 } });
    await timed.addInitScript(() => {
      window.__cardTimes = { inserted: null, armed: null };
      new MutationObserver(() => {
        const times = window.__cardTimes;
        if (times.inserted == null && document.querySelector("article.card")) {
          times.inserted = performance.now();
        }
        if (times.armed == null && document.querySelector("article.card .hold-hint.armed")) {
          times.armed = performance.now();
        }
      }).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
    });
    await timed.addInitScript(...stub({ label: "card", snapshot }));
    await timed.goto(origin, { waitUntil: "networkidle" });
    await timed.getByRole("button", { name: "Approve draft" }).waitFor();
    await timed.waitForFunction(() => document.activeElement?.matches("article.card") ?? false, null, { timeout: 3000 }).catch(() => {});
    const focused = await timed.evaluate(() => ({
      card: document.activeElement?.matches("article.card") ?? false,
      what: document.activeElement?.tagName + "." + (document.activeElement?.className ?? ""),
    }));
    if (!focused.card) {
      throw new Error(`the card window opened with focus on ${focused.what}, not the card`);
    }
    await timed.waitForFunction(() => window.__cardTimes.armed != null, null, { timeout: 8000 });
    const times = await timed.evaluate(() => window.__cardTimes);
    const dwell = times.armed - times.inserted;
    // 520 rise + 800 dwell; a little slack for frame timing.
    if (dwell < 1260) {
      throw new Error(`the seen lock armed ${Math.round(dwell)}ms after insertion; the dwell must start when the rise lands`);
    }
    // UX 6: Enter or Space on Approve with a modifier held never approves.
    await timed.locator("article.card button.approve").focus();
    for (const chord of ["Control+Enter", "Meta+Enter", "Alt+Enter", "Shift+Enter", "Control+Space", "Shift+Space"]) {
      await timed.keyboard.press(chord);
    }
    await timed.waitForTimeout(700);
    if ((await signs(timed)) !== 0) {
      throw new Error("a modified Enter/Space on Approve signed (bypassing the C4 hold)");
    }
    await timed.close();
  }

  // Blind input, reserved hint row, native sight, then Hello in flight.
  const page = await openCard(browser, { snapshot, helloMs: 700 });
  // Measure once the 520ms rise has landed (the dwell runs 800ms after that).
  await page.waitForFunction(() => document.querySelector("article.card")?.getAnimations().length === 0, null, {
    timeout: 3000,
  });
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
    approvedFace: document.querySelectorAll("article.card button.approve .face-done.show").length,
    idleFace: document.querySelectorAll("article.card button.approve .face-idle:not(.gone)").length,
  }));
  if (hello.buttons !== 0 || hello.hint) {
    throw new Error(`Hello pending drew controls ${JSON.stringify(hello)}`);
  }
  // UX 5: during Hello Approve still reads "Approve draft"; "Approved" only after success.
  if (hello.approvedFace !== 0 || hello.idleFace === 0) {
    throw new Error(`Approve said Approved during Hello ${JSON.stringify(hello)}`);
  }
  if ((await signs(page)) !== 1) {
    throw new Error("Enter on the armed Approve did not sign exactly once");
  }
  await page.waitForFunction(() => !document.querySelector("article.card .quiet .hello"), null, { timeout: 3000 });
  await page.close();

  // CD (LOOK 8.3 rows 125-128, INTERACTIONS S5 70-73): after Hello the card
  // stays glass with the drawn check and "Approved" for the whole 6s undo
  // window, Deny's slot counting Undo down. Only when the window closes does
  // the glass set to paper and fold into the receipt, which has no Undo.
  // The card video: the face swap starts at 60fps f246 and the paperize at
  // f613 (367 frames); its countdown is the 6s window = 360 frames, the other
  // ~7 frames are the video's commit lead. The app's window is the daemon's
  // undo_until (decided_at + 6000ms), so the decided glass must span
  // 360 frames (6.0s) within a frame-timing tolerance.
  const WINDOW_FRAMES = 360;
  async function sampleCard(page) {
    await page.evaluate(() => {
      window.__cardFrames = [];
      window.__sampling = true;
      const sample = () => {
        const card = document.querySelector("article.card");
        const face = card?.querySelector("button.approve .face-done");
        const style = card ? getComputedStyle(card) : null;
        window.__cardFrames.push({
          t: performance.now(),
          face: face ? Number(getComputedStyle(face).opacity) : 0,
          check: Boolean(face?.querySelector("svg")),
          label: face?.textContent?.trim() ?? "",
          glass: Boolean(card?.classList.contains("glass")),
          receipt: Boolean(card?.classList.contains("receipt")),
          undo: card?.querySelector("button.undo .face-done.show")?.textContent?.replace(/\s+/g, "") ?? null,
          buttons: card?.querySelectorAll("button").length ?? 0,
          height: card ? card.getBoundingClientRect().height : 0,
          opacity: style ? Number(style.opacity) : 0,
        });
        if (window.__sampling && window.__cardFrames.length < 1200) {
          requestAnimationFrame(sample);
        }
      };
      requestAnimationFrame(sample);
    });
  }

  for (const motion of ["no-preference", "reduce"]) {
    const page = await browser.newPage({ viewport: { width: 560, height: 760 } });
    await page.emulateMedia({ reducedMotion: motion });
    await page.addInitScript(...stub({ label: "card", snapshot, helloMs: 300, decides: true }));
    await page.goto(origin, { waitUntil: "networkidle" });
    await page.getByRole("button", { name: "Approve draft" }).waitFor();
    await arm(page);
    await sampleCard(page);
    await page.locator("article.card button.approve").focus();
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => window.__cardFrames.some((frame) => frame.receipt), null, { timeout: 9000 });
    await page.waitForTimeout(900);
    await page.evaluate(() => {
      window.__sampling = false;
    });
    const frames = await page.evaluate(() => window.__cardFrames);
    const swap = frames.findIndex((frame) => frame.face > 0.01);
    const full = frames.findIndex((frame) => frame.face >= 0.99);
    const paper = frames.findIndex((frame, i) => i > swap && !frame.glass);
    const filed = frames.findIndex((frame) => frame.receipt);
    if (swap < 0 || full < 0 || paper < 0 || filed < 0) {
      throw new Error(`${motion}: the decided face never showed or never filed ${JSON.stringify({ swap, full, paper, filed })}`);
    }
    const glassSpan = frames.slice(swap, paper);
    const spanMs = Math.round(frames[paper].t - frames[swap].t);
    const spanFrames = paper - swap;
    const fullFrames = paper - full;
    if (glassSpan.some((frame) => !frame.glass || frame.receipt || !frame.check || frame.label !== "Approved")) {
      throw new Error(`${motion}: the decided glass did not hold the check and "Approved" throughout`);
    }
    if (spanMs < 5900 || spanMs > 6250) {
      throw new Error(`${motion}: the decided glass held ${spanFrames} frames (${spanMs}ms); the undo window is ${WINDOW_FRAMES} frames (6000ms)`);
    }
    // Undo counts down on the card, not the receipt.
    const undoLabels = [...new Set(glassSpan.map((frame) => frame.undo))];
    if (!undoLabels.includes("Undo6s") || !undoLabels.includes("Undo1s") || undoLabels.includes(null)) {
      throw new Error(`${motion}: the card's Undo countdown ${JSON.stringify(undoLabels)}`);
    }
    // The fold: frames between the glass and the settled receipt where the
    // height (or, reduced, the 160ms crossfade) is between its ends.
    const settled = frames[frames.length - 1];
    const from = frames[paper - 1].height;
    const fold = frames.slice(filed).filter((frame) =>
      motion === "reduce"
        ? frame.opacity > 0.001 && frame.opacity < 0.999
        : Math.abs(frame.height - settled.height) > 0.5 && Math.abs(frame.height - from) > 0.5,
    ).length;
    if (fold <= 0) {
      throw new Error(`${motion}: foldFrames 0 (the card cut to the receipt)`);
    }
    if (settled.buttons !== 0 || settled.undo !== null || !settled.receipt) {
      throw new Error(`${motion}: the filed receipt carries a control ${JSON.stringify(settled)}`);
    }
    if ((await page.getByRole("button", { name: /Undo/ }).count()) !== 0) {
      throw new Error(`${motion}: the filed receipt has an Undo`);
    }
    console.log(
      `card-window: decided glass ${spanFrames} frames (${spanMs}ms, face fully shown ${fullFrames} frames), paperize ${filed - paper} frames, foldFrames ${fold}, receipt buttons ${settled.buttons} (${motion})`,
    );
    await page.close();
  }

  // INTERACTIONS S5 + SD condition 2: Ctrl/Cmd+Z undoes only while the card is
  // glass, visible and focused, and passes undoSight. Blurred, hidden,
  // minimized, collapsed, covered or unfocused: no undo_decision.
  {
    const page = await browser.newPage({ viewport: { width: 560, height: 760 } });
    await page.addInitScript(...stub({ label: "card", snapshot, helloMs: 200, decides: true }));
    await page.goto(origin, { waitUntil: "networkidle" });
    await page.getByRole("button", { name: "Approve draft" }).waitFor();
    await arm(page);
    await page.locator("article.card button.approve").focus();
    await page.keyboard.press("Enter");
    await page.locator("article.card button.undo").waitFor({ timeout: 3000 });
    const undos = () => page.evaluate(() => window.__invokes.filter((entry) => entry.command === "undo_decision").length);
    const stillDecided = () =>
      page.evaluate(() => Boolean(document.querySelector("article.card.glass button.undo")) && Boolean(document.querySelector("article.card .face-done.show")));
    const focusCard = () => page.evaluate(() => document.querySelector("article.card")?.focus());
    const cases = [
      ["blurred", () => page.evaluate(() => window.dispatchEvent(new Event("blur"))), () => page.evaluate(() => window.dispatchEvent(new Event("focus")))],
      [
        "hidden",
        () =>
          page.evaluate(() => {
            Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
            document.dispatchEvent(new Event("visibilitychange"));
          }),
        () =>
          page.evaluate(() => {
            delete document.visibilityState;
            document.dispatchEvent(new Event("visibilitychange"));
          }),
      ],
      [
        "minimized",
        () => page.evaluate(() => window.dispatchEvent(new CustomEvent("dasdevbot:native-sight", { detail: { visible: false } }))),
        () => page.evaluate(() => window.dispatchEvent(new CustomEvent("dasdevbot:native-sight", { detail: { visible: true } }))),
      ],
      [
        "collapsed",
        () =>
          page.evaluate(() => {
            const main = document.querySelector("main.card-window");
            main.classList.add("win");
            main.dataset.shellForm = "pill";
          }),
        () =>
          page.evaluate(() => {
            const main = document.querySelector("main.card-window");
            main.classList.remove("win");
            delete main.dataset.shellForm;
          }),
      ],
      [
        "covered",
        () => page.evaluate(() => document.querySelector("main.card-window").setAttribute("inert", "")),
        () => page.evaluate(() => document.querySelector("main.card-window").removeAttribute("inert")),
      ],
      ["unfocused", () => page.evaluate(() => document.querySelector("main.card-window").focus()), async () => {}],
    ];
    const refused = [];
    for (const [name, enter, leave] of cases) {
      await focusCard();
      await enter();
      await page.keyboard.press("Control+z");
      await page.keyboard.press("Meta+z");
      await page.waitForTimeout(120);
      if ((await undos()) !== 0 || !(await stillDecided())) {
        throw new Error(`Ctrl/Cmd+Z undid while the card was ${name}`);
      }
      await leave();
      refused.push(name);
    }
    // Glass, visible, focused: one Ctrl+Z undoes through undo_decision.
    await focusCard();
    await page.keyboard.press("Control+z");
    await page.waitForFunction(() => document.querySelector("article.card .face-idle:not(.gone)") && !document.querySelector("article.card button.undo"), null, { timeout: 2000 });
    const undoneAt = Date.now();
    const restored = await page.evaluate(() => ({
      undos: window.__invokes.filter((entry) => entry.command === "undo_decision").length,
      approve: document.querySelector("article.card button.approve .face-idle:not(.gone)")?.textContent?.trim() ?? "",
      deny: document.querySelector("article.card button.deny .face-idle:not(.gone)")?.textContent?.trim() ?? "",
      faceDone: document.querySelectorAll("article.card .face-done.show").length,
      armed: Boolean(document.querySelector("article.card .hold-hint.armed")),
      focus: document.activeElement?.matches("article.card") ?? false,
    }));
    if (restored.undos !== 1 || restored.faceDone !== 0 || !restored.approve.startsWith("Approve draft") || restored.deny !== "Deny draft") {
      throw new Error(`Ctrl+Z did not restore the waiting card ${JSON.stringify(restored)}`);
    }
    if (restored.armed || !restored.focus) {
      throw new Error(`the seen lock was not re-armed from zero after undo ${JSON.stringify(restored)}`);
    }
    // The check retracts (ink back to 0) and the lock re-arms only after a fresh dwell.
    await page.waitForFunction(() => document.querySelector("article.card .hold-hint.armed"), null, { timeout: 4000 });
    const dwell = Date.now() - undoneAt;
    const ink = await page.evaluate(() => document.querySelector("article.card button.approve > .ink")?.style.clipPath ?? "");
    if (dwell < 700 || !/inset\(0px 100(\.000)?% 0px 0px\)/.test(ink)) {
      throw new Error(`after undo: re-armed in ${dwell}ms, ink ${ink}`);
    }
    // A second approve is a new signed decision with a new event id.
    await page.locator("article.card button.approve").focus();
    await page.keyboard.press("Enter");
    await page.locator("article.card.receipt").waitFor({ timeout: 9000 });
    await page.waitForTimeout(800);
    const second = await page.evaluate(() => ({
      signs: window.__invokes.filter((entry) => entry.command === "sign_decision").length,
      stamp: document.querySelector("article.card .stamp")?.textContent ?? "",
    }));
    if (second.signs !== 2 || !second.stamp.includes("2222") || second.stamp.includes("1111")) {
      throw new Error(`the second approve did not file under a new event id ${JSON.stringify(second)}`);
    }
    console.log(
      `card-window: Ctrl/Cmd+Z refused while ${refused.join(", ")}; focused undo restored "Approve draft" + Deny, seen lock re-armed after ${dwell}ms, re-approve filed ${second.stamp.replace(/\s+/g, " ").trim()} (sign_decision x${second.signs}, undo_decision x${restored.undos})`,
    );
    await page.close();
  }

  // INTERACTIONS S5: the roster's waiting dot leaves for the undo window
  // ("approved · undo Ns", ink-3, no dot) and comes back on undo.
  {
    const main = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await main.addInitScript(...stub({ label: "main", snapshot }));
    await main.goto(origin, { waitUntil: "networkidle" });
    const roster = () =>
      main.evaluate(() => ({
        dot: document.querySelectorAll(".roster .wdot").length,
        lines: [...document.querySelectorAll(".roster .sub")].map((el) => el.textContent?.trim() ?? "").filter(Boolean),
      }));
    await main.waitForFunction(() => document.querySelectorAll(".roster .wdot").length === 1, null, { timeout: 4000 });
    const at = Date.now();
    await main.evaluate(
      ({ snapshot, at }) =>
        window.__TAURI_INTERNALS__.setSnapshot({
          ...snapshot,
          approvals: snapshot.approvals.map((a) => ({ ...a, status: "approved", decided_at: at, decision_event_id: "ev_11111111", undo_until: at + 6000 })),
        }),
      { snapshot, at },
    );
    await main.waitForFunction(() => document.querySelectorAll(".roster .wdot").length === 0, null, { timeout: 4000 });
    const during = await roster();
    if (!during.lines.some((line) => /^approved · undo \ds$/.test(line))) {
      throw new Error(`roster during the undo window ${JSON.stringify(during)}`);
    }
    await main.evaluate(({ snapshot }) => window.__TAURI_INTERNALS__.setSnapshot(snapshot), { snapshot });
    await main.waitForFunction(() => document.querySelectorAll(".roster .wdot").length === 1, null, { timeout: 4000 });
    const after = await roster();
    if (!after.lines.includes("waiting on you")) {
      throw new Error(`roster after undo ${JSON.stringify(after)}`);
    }
    console.log(`card-window: roster dot ${during.dot} during the window (${during.lines.join(" | ")}), ${after.dot} after undo`);
    await main.close();
  }

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

  // C1 from the real daemon shape: the main window renders the denied,
  // committed destructive approval as the flat row under its gate-denied event;
  // it takes no waiting dot and no pill count; the card window shows no card.
  {
    const flat = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await flat.addInitScript(
      ...stub({ label: "main", snapshot: { ...snapshot, approvals: [destructive], events: forceEvents } }),
    );
    await flat.goto(origin, { waitUntil: "networkidle" });
    await flat.locator("article.card.flat").first().waitFor();
    const row = await flat.evaluate(() => {
      const card = document.querySelector("article.card.flat");
      return {
        text: card?.textContent ?? "",
        buttons: card?.querySelectorAll("button").length ?? -1,
        tab: card?.getAttribute("tabindex"),
        approve: document.querySelectorAll("button.approve, [data-waiting]").length,
        nothingWaiting: document.body.textContent?.includes("Nothing is waiting.") ?? false,
        gateRowShown: [...document.querySelectorAll(".event .who")].some((who) => /gate\.denied/.test(who.textContent ?? "")),
      };
    });
    // CD ruling c: "Builder wanted to force-push <ref>. Destructive actions are off in this build." + the mono command.
    // The name comes from the approval, not the copy: the stub says Builder, so the row must.
    if (
      !/\bBuilder wanted to force-push phase0\. Destructive actions are off in this build\./.test(row.text) ||
      !row.text.includes("git push --force origin phase0")
    ) {
      throw new Error(`main: destructive row copy ${JSON.stringify(row)}`);
    }
    if (row.buttons !== 0 || row.tab != null || row.approve !== 0) {
      throw new Error(`main: destructive row has controls ${JSON.stringify(row)}`);
    }
    if (row.nothingWaiting) {
      throw new Error(`main: the C1 row did not replace the quiet line ${JSON.stringify(row)}`);
    }
    if (row.gateRowShown) {
      throw new Error(`main: the gate.denied event shows as a bare row ${JSON.stringify(row)}`);
    }
    // Destructive stays out of the waiting dot and the pill count.
    const waiting = await flat.evaluate(() => ({
      dot: document.querySelectorAll(".roster .wdot").length,
      count: document.querySelector(".cl.pill .n")?.textContent?.trim() ?? null,
      waitingLine: [...document.querySelectorAll(".roster .sub")].some((el) => el.textContent?.includes("waiting on you")),
    }));
    if (waiting.dot !== 0 || waiting.waitingLine || waiting.count !== "0") {
      throw new Error(`main: destructive took the waiting dot or the pill count ${JSON.stringify(waiting)}`);
    }
    await flat.close();

    const cardPage = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await cardPage.addInitScript(
      ...stub({ label: "card", snapshot: { ...snapshot, approvals: [destructive], events: forceEvents } }),
    );
    await cardPage.goto(origin, { waitUntil: "networkidle" });
    await cardPage.waitForTimeout(1500);
    const inCard = await cardPage.evaluate(() => ({
      cards: document.querySelectorAll("article.card:not(.flat)").length,
      approve: document.querySelectorAll("button.approve").length,
    }));
    if (inCard.cards !== 0 || inCard.approve !== 0) {
      throw new Error(`card: a destructive approval opened a deciding card ${JSON.stringify(inCard)}`);
    }
    await cardPage.close();
  }

  // Main window: no decision control in any form.
  const main = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await main.addInitScript(...stub({ label: "main", snapshot }));
  await main.goto(origin, { waitUntil: "networkidle" });
  await main.locator("[data-waiting='ap_card']").waitFor();
  if ((await main.locator("button.approve, article.card[tabindex]").count()) !== 0) {
    throw new Error("the main window drew a decision control");
  }
  // UID 5: Alt+Down in the main window focuses the oldest waiting step.
  await main.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
  });
  await main.keyboard.press("Alt+ArrowDown");
  await main.waitForTimeout(200);
  if (!(await main.evaluate(() => document.activeElement?.matches("[data-waiting='ap_card']") ?? false))) {
    const at = await main.evaluate(() => document.activeElement?.outerHTML.slice(0, 80) ?? "none");
    throw new Error(`main: Alt+Down did not focus the waiting step (focus on ${at})`);
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
