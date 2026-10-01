import {
  STORY,
  storyHistory,
  storyStep,
  type StepSpec,
} from "./fixture";
import { tokenize } from "./rich";
import type { ThreadFrame, ThreadNode, ToolBlockView, ToolStepView } from "./types";

export interface PlayHost {
  frame: ThreadFrame;
  sleep: (ms: number) => Promise<void>;
  stopped: () => boolean;
  reduced: () => boolean;
  holdApprove: () => Promise<void>;
}

const ASK = "turn-ask";
const STAGE_MS = 520;
const SOFT_MS = 360;
const BASE_MS = 240;
const DRAW_MS = 300;

const marks: Record<string, number> = {};
let origin = 0;

/** Virtual-clock stamps, same names as thread.html `window.__thread.marks`. */
export function storyMarks(): Record<string, number> {
  return marks;
}

function mark(name: string): void {
  marks[name] = Math.round(performance.now() - origin);
}

function clearMarks(): void {
  for (const key of Object.keys(marks)) {
    delete marks[key];
  }
  origin = performance.now();
}

/** Same mulberry32-style mixer the prototype uses, so one seed is one timing. */
function rng(seed: number): () => number {
  let state = seed;
  return () => {
    state |= 0;
    state = (state + 0x6d2b79f5) | 0;
    let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
    mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

export async function playStory(host: PlayHost): Promise<void> {
  const frame = host.frame;
  const reduced = host.reduced();
  clearMarks();
  frame.chip = "Reviewer";
  frame.placeholder = "Message Reviewer";
  frame.draft = "";
  frame.lit = false;
  frame.cursor = null;
  frame.nodes = storyHistory();
  await host.sleep(300);
  if (host.stopped()) {
    return;
  }

  mark("typeStart");
  const per = 900 / STORY.typed.length;
  const typed = rng(7);
  for (let i = 1; i <= STORY.typed.length; i += 1) {
    frame.draft = STORY.typed.slice(0, i);
    const gap = Math.round(per * (0.6 + typed() * 0.8));
    await host.sleep(gap);
    if (host.stopped()) {
      return;
    }
  }
  mark("typeEnd");
  await host.sleep(200);
  await host.sleep(110);
  mark("sendStart");
  frame.draft = "";
  frame.nodes = [
    ...frame.nodes,
    { type: "user", id: "user-send", text: STORY.typed, time: "10:42 AM", hlc: "" },
  ];
  await landUser(host);
  if (host.stopped()) {
    return;
  }
  mark("sendLanded");
  await host.sleep(200);

  frame.nodes = [
    ...frame.nodes,
    {
      type: "turn",
      id: ASK,
      agent: "reviewer",
      name: "Reviewer",
      time: "10:42 AM",
      parts: [{ type: "tools", block: { id: "tools-main", folded: false, open: false, steps: [] } }],
      approval: null,
    },
  ];
  mark("turnIn");
  await host.sleep(200);
  if (!(await runSteps(host))) {
    return;
  }

  mark("streamStart");
  await streamInto(host, ASK, STORY.reply, "bub-ask", 11);
  if (host.stopped()) {
    return;
  }
  mark("streamEnd");

  const ask = turnById(frame, ASK);
  if (ask) {
    ask.parts = [
      ...ask.parts,
      {
        type: "tools",
        block: {
          id: "tools-post",
          folded: false,
          open: false,
          steps: [waitingStep(STORY.post)],
        },
      },
    ];
  }
  mark("riskyStep");
  await host.sleep((reduced ? 160 : SOFT_MS) + 60);
  mark("cardLift");
  const lifting = turnById(frame, ASK);
  if (lifting) {
    lifting.approval = pendingAsk();
  }
  await host.sleep(0);
  await host.sleep(reduced ? 160 : 150 + STAGE_MS);
  if (host.stopped()) {
    return;
  }
  mark("cardLanded");
  await host.sleep(160);
  await showCursor(host);
  const approve = center('article.card[tabindex="0"] button.approve', 0.56, 0.58);
  const deny = center('article.card[tabindex="0"] button.deny', 0.5, 0.55);
  if (approve) {
    await moveCursor(host, approve.x, approve.y, 600);
  }
  await host.sleep(90);
  if (host.stopped()) {
    return;
  }
  mark("holdStart");
  await host.holdApprove();
  if (host.stopped()) {
    return;
  }
  mark("hello");
  await host.sleep(16.667);

  await host.sleep(500);
  if (deny) {
    await moveCursor(host, deny.x, deny.y, 520);
  }
  await host.sleep(620);
  const form = document.querySelector("form.composer");
  if (form instanceof HTMLElement) {
    const box = form.getBoundingClientRect();
    await moveCursor(host, box.left + 140, box.top + 30, 700);
    mark("specStart");
    await moveCursor(host, box.right - 170, box.top + 22, 1500);
    await moveCursor(host, box.left + 380, box.top + 34, 800);
    await moveCursor(host, box.right + 90, box.top - 150, 600);
  }
  mark("specEnd");
  frame.cursor = frame.cursor ? { ...frame.cursor, visible: false } : null;
  frame.lit = false;
  await host.sleep(reduced ? 120 : 160);
  while (!host.stopped() && !document.querySelector('article.card[data-filed="1"]')) {
    await host.sleep(16.667);
  }
  if (host.stopped()) {
    return;
  }
  mark("filed");
  const foldedTools = mainTools(frame);
  if (foldedTools) {
    const summary = center(".thread .sum", 0.5, 0.5);
    if (summary) {
      host.frame.cursor = { x: summary.x + 220, y: summary.y + 140, visible: true };
      await moveCursor(host, summary.x - 30, summary.y + 2, 460);
    } else {
      await host.sleep(460);
    }
    mark("expand");
    foldedTools.open = true;
    await host.sleep(140);
    await host.sleep(760);
    await host.sleep(140);
    mark("collapse");
    foldedTools.open = false;
    await host.sleep(reduced ? 0 : SOFT_MS);
    await host.sleep(160);
    frame.cursor = null;
    frame.lit = false;
  }
  if (host.stopped()) {
    return;
  }

  frame.nodes = [
    ...frame.nodes,
    {
      type: "handoff",
      id: "hand-now",
      fromId: "ev_4b1e07",
      toId: "ev_4c83d5",
      label: "Reviewer hands off to Builder",
      drawn: true,
    },
  ];
  mark("handoff");
  frame.chip = "Builder";
  frame.placeholder = "Message Builder";
  await host.sleep(reduced ? 160 : 220 + BASE_MS);
  if (!reduced) {
    await host.sleep(120);
  }
  await host.sleep(160);
  if (host.stopped()) {
    return;
  }
  mark("builderIn");

  frame.nodes = [
    ...frame.nodes,
    {
      type: "turn",
      id: "turn-builder",
      agent: "builder",
      name: "Builder",
      time: "10:42 AM",
      parts: [],
      approval: null,
    },
  ];
  await host.sleep(220);
  await streamInto(host, "turn-builder", STORY.replyBuilder, "bub-builder", 5);
  mark("end");
}

async function runSteps(host: PlayHost): Promise<boolean> {
  const reduced = host.reduced();
  const foldAt = (reduced ? 0 : DRAW_MS) + 120;
  for (let index = 0; index < STORY.steps.length; index += 1) {
    const spec = STORY.steps[index];
    const tools = mainTools(host.frame);
    if (!spec || !tools) {
      continue;
    }
    mark(`step${index + 1}`);
    finishLive(tools);
    const step = storyStep(spec, "live", index);
    step.elapsed = "0.0s";
    tools.steps = [...tools.steps, step];
    let done = 0;
    while (done < spec.ms) {
      const untilFold = index === 3 && !tools.folded ? foldAt - done : spec.ms;
      const slice = Math.min(100, spec.ms - done, Math.max(0, untilFold));
      if (slice <= 0) {
        break;
      }
      await host.sleep(slice);
      done += slice;
      const live = mainTools(host.frame)?.steps.find((item) => item.id === step.id);
      if (live && live.phase === "live") {
        const secs = Number.parseFloat(spec.secs);
        live.elapsed = ((secs * done) / spec.ms).toFixed(1) + "s";
      }
      if (index === 3 && !tools.folded && done >= foldAt) {
        tools.folded = true;
        mark("fold");
      }
      if (host.stopped()) {
        return false;
      }
    }
  }
  const folded = mainTools(host.frame);
  if (folded) {
    finishLive(folded);
  }
  await host.sleep(reduced ? 120 : 60 + DRAW_MS);
  // The prototype resolves the pen-draw and fold promises on the clock settle
  // after those durations, which lands 26ms later at a 16.667ms step.
  await host.sleep(reduced ? 0 : SOFT_MS + 26);
  mark("toolsDone");
  return !host.stopped();
}

async function landUser(host: PlayHost): Promise<void> {
  await host.sleep(0);
  const field = document.querySelector("textarea");
  const bubble = document.querySelector(".thread .me:last-of-type .bub");
  if (!(field instanceof HTMLElement) || !(bubble instanceof HTMLElement) || host.reduced()) {
    await host.sleep(host.reduced() ? 160 : STAGE_MS);
    return;
  }
  const from = field.getBoundingClientRect();
  const to = bubble.getBoundingClientRect();
  const easing =
    getComputedStyle(document.documentElement).getPropertyValue("--ease-out").trim() ||
    "cubic-bezier(0.22, 1, 0.36, 1)";
  bubble.animate(
    [
      { transform: `translate(${from.left - to.left}px, ${from.top - to.top}px)` },
      { transform: "none" },
    ],
    { duration: STAGE_MS, easing, fill: "backwards" },
  );
  await host.sleep(STAGE_MS);
}

function turnById(frame: ThreadFrame, id: string): Extract<ThreadNode, { type: "turn" }> | null {
  const node = frame.nodes.find((item) => item.id === id);
  if (!node || node.type !== "turn") {
    return null;
  }
  return node;
}

function toolBlock(turn: Extract<ThreadNode, { type: "turn" }>, id: string): ToolBlockView | null {
  for (const part of turn.parts) {
    if (part.type === "tools" && part.block.id === id) {
      return part.block;
    }
  }
  return null;
}

function mainTools(frame: ThreadFrame): ToolBlockView | null {
  const turn = turnById(frame, ASK);
  return turn ? toolBlock(turn, "tools-main") : null;
}

function finishLive(tools: ToolBlockView): void {
  tools.steps = tools.steps.map((step) => {
    if (step.phase !== "live") {
      return step;
    }
    const spec = STORY.steps.find((item) => item.say === step.say);
    return { ...step, phase: "done", elapsed: spec?.secs ?? step.elapsed };
  });
}

function waitingStep(spec: StepSpec): ToolStepView {
  return { ...storyStep(spec, "waiting", 9), elapsed: "waiting on you" };
}

function pendingAsk() {
  return {
    id: "ap_ask",
    job_id: "job_fixture",
    agent_id: "reviewer",
    agent_name: "Reviewer",
    thread_id: "thread_fixture",
    effect_class: "external",
    action: "post_pr_comment",
    purpose: "Leave one review comment flagging an unhandled error path in the session handoff.",
    draft:
      "If refresh() rejects on a 401, the handoff lock is never released. Wrap it in try/finally so the next session can take the lock.",
    evidence: {
      repo: "DasVR/NIL",
      pr: "#212 handoff: release lock on refresh",
      ref: "phase0 @ a41c9e2",
      event_id: "ev_3f9a2c",
      kind: "tool.request",
    },
    evidence_text: "repo DasVR/NIL",
    status: "pending",
    provider: "mock",
    model: "reviewer-small",
    usage_kind: "estimated",
    input_tokens: 2418,
    output_tokens: 212,
    micro_usd: 431,
    created_at: Date.now(),
    expires_at: null,
    decided_at: null,
    decision_event_id: null,
    reason: null,
    committed: false,
    undo_until: null,
  };
}

export function fileAsk(frame: ThreadFrame): void {
  const turn = turnById(frame, ASK);
  if (!turn?.approval) {
    return;
  }
  turn.approval = {
    ...turn.approval,
    status: "approved",
    committed: true,
    undo_until: null,
    decided_at: turn.approval.decided_at ?? Date.now(),
    decision_event_id: "ev_4b1e07",
  };
  const step = toolBlock(turn, "tools-post")?.steps[0];
  if (step) {
    step.phase = "done";
    step.elapsed = STORY.post.secs;
  }
}

async function streamInto(
  host: PlayHost,
  id: string,
  text: string,
  bubbleId: string,
  seed: number,
): Promise<void> {
  const turn = turnById(host.frame, id);
  if (!turn) {
    return;
  }
  const bubble = { id: bubbleId, text, shown: host.reduced() ? text : "" };
  turn.parts = [...turn.parts, { type: "bubble", bubble }];
  if (host.reduced()) {
    await host.sleep(160);
    return;
  }
  await host.sleep(140);
  const roll = rng(seed);
  let shown = "";
  for (const token of tokenize(text)) {
    const live = turnById(host.frame, id);
    const part = live?.parts.find((item) => item.type === "bubble" && item.bubble.id === bubbleId);
    if (!part || part.type !== "bubble") {
      return;
    }
    shown += token;
    part.bubble.shown = shown;
    const pause = 20 + Math.round(roll() * 26) + (/[.,]\s*$/.test(token) ? 50 : 0);
    await host.sleep(pause);
    if (host.stopped()) {
      return;
    }
  }
  await host.sleep(200);
}

const easeInOut = bezier(0.65, 0, 0.35, 1);

function bezier(x1: number, y1: number, x2: number, y2: number): (x: number) => number {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const sampleX = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sampleY = (t: number) => ((ay * t + by) * t + cy) * t;
  return (x: number) => {
    if (x <= 0) {
      return 0;
    }
    if (x >= 1) {
      return 1;
    }
    let lo = 0;
    let hi = 1;
    let t = x;
    for (let i = 0; i < 40; i += 1) {
      if (sampleX(t) < x) {
        lo = t;
      } else {
        hi = t;
      }
      t = (lo + hi) / 2;
    }
    return sampleY(t);
  };
}

function center(selector: string, fx: number, fy: number): { x: number; y: number } | null {
  const el = document.querySelector(selector);
  if (!(el instanceof HTMLElement)) {
    return null;
  }
  const box = el.getBoundingClientRect();
  return { x: box.left + box.width * fx, y: box.top + box.height * fy };
}

function placeSheen(host: PlayHost, x: number, y: number): void {
  const form = document.querySelector("form.composer");
  if (!(form instanceof HTMLElement)) {
    return;
  }
  const box = form.getBoundingClientRect();
  const inside = x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
  host.frame.lit = inside;
  if (inside) {
    host.frame.sheenX = x - box.left;
  }
}

function tween(
  host: PlayHost,
  ms: number,
  ease: (t: number) => number,
  apply: (p: number) => void,
): Promise<void> {
  if (host.reduced() || ms <= 0) {
    return host.sleep(ms).then(() => {
      apply(1);
    });
  }
  const start = performance.now();
  apply(ease(0));
  return new Promise((resolve) => {
    const frame = () => {
      if (host.stopped()) {
        resolve();
        return;
      }
      const p = Math.min(1, (performance.now() - start) / ms);
      apply(ease(p));
      if (p < 1) {
        requestAnimationFrame(frame);
        return;
      }
      resolve();
    };
    requestAnimationFrame(frame);
  });
}

async function showCursor(host: PlayHost): Promise<void> {
  const approve = center('article.card[tabindex="0"] button.approve', 0.56, 0.58);
  const x = approve ? approve.x + 180 : 200;
  const y = approve ? approve.y + 160 : 200;
  host.frame.cursor = { x, y, visible: false };
  await host.sleep(0);
  host.frame.cursor = { x, y, visible: true };
  await host.sleep(host.reduced() ? 120 : 160);
}

async function moveCursor(host: PlayHost, x: number, y: number, ms: number): Promise<void> {
  const from = host.frame.cursor ?? { x, y, visible: true };
  await tween(host, ms, easeInOut, (p) => {
    const cx = from.x + (x - from.x) * p;
    const cy = from.y + (y - from.y) * p;
    host.frame.cursor = { x: cx, y: cy, visible: true };
    placeSheen(host, cx, cy);
  });
}

export function undoAsk(frame: ThreadFrame): void {
  const turn = turnById(frame, ASK);
  if (!turn?.approval) {
    return;
  }
  turn.approval = {
    ...turn.approval,
    status: "pending",
    committed: false,
    undo_until: null,
    decided_at: null,
    decision_event_id: null,
    reason: null,
  };
}

export function settleAsk(frame: ThreadFrame, decision: "approve" | "deny"): void {
  const turn = turnById(frame, ASK);
  if (!turn?.approval) {
    return;
  }
  const approved = decision === "approve";
  turn.approval = {
    ...turn.approval,
    status: approved ? "approved" : "denied",
    committed: false,
    undo_until: Date.now() + 6_000,
    decided_at: Date.now(),
    decision_event_id: "ev_4b1e07",
  };
  const step = toolBlock(turn, "tools-post")?.steps[0];
  if (step) {
    step.phase = "done";
    step.elapsed = approved ? "approved · undo open" : "denied · undo open";
  }
}
