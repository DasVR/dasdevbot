import type { Approval } from "../api";
import type { AgentKey, ThreadFrame, ThreadNode, ToolBlockView, ToolStepView, TurnPart } from "./types";

export const SHOTS = ["send", "tools", "approval", "receipt", "handoff", "composer", "deny"] as const;
export type ShotName = (typeof SHOTS)[number];

export function isShot(value: string | null): value is ShotName {
  return SHOTS.some((shot) => shot === value);
}

const REPLY =
  "Found one thing worth raising. If `refresh()` rejects on a 401, the handoff lock is never released, so the next session can’t take it. I drafted one comment suggesting a `try/finally`. Posting to GitHub needs your OK.";

const REPLY_BUILDER =
  "Picked up ev_4c83d5. I’ll re-run the handoff tests once CI on #212 is green and report back here.";

const USER_SEND = "Pushed. Review the handoff fix, one comment max.";

export interface StepSpec {
  say: string;
  detail: string;
  secs: string;
  ms: number;
  trace: ToolStepView["trace"];
}

const STEPS: StepSpec[] = [
  {
    say: "Reading the diff",
    detail: "git diff a41c9e2~1 a41c9e2 --stat · 3 files, +16 −6",
    secs: "3.2s",
    ms: 700,
    trace: "reading",
  },
  {
    say: "Tracing where the handoff lock is released",
    detail: 'rg -n "release_lock" src/ · 4 hits',
    secs: "1.9s",
    ms: 660,
    trace: "reading",
  },
  {
    say: "Running the core tests",
    detail: "cargo test -p core · 41 passed, 0 failed",
    secs: "24.6s",
    ms: 760,
    trace: "reading",
  },
  {
    say: "Checking why refresh() can reject",
    detail: "git log -L :refresh:src/handoff.ts · 2 commits",
    secs: "3.1s",
    ms: 640,
    trace: "reading",
  },
  {
    say: "Drafting one review comment",
    detail: "reviewer-small · 2,418 in / 212 out · $0.000431",
    secs: "9.2s",
    ms: 700,
    trace: "writing",
  },
];

const POST: StepSpec = {
  say: "Posting the comment to DasVR/NIL #212",
  detail: "gh pr comment 212 · ev_3f9a2c · external",
  secs: "0.6s",
  ms: 520,
  trace: "writing",
};

function liveStep(spec: StepSpec, index: number, elapsed: string): ToolStepView {
  return { ...storyStep(spec, "live", index), elapsed };
}

export function storyStep(spec: StepSpec, phase: ToolStepView["phase"], index: number): ToolStepView {
  return {
    id: `step-${index}-${phase}`,
    say: spec.say,
    detail: spec.detail,
    elapsed: phase === "live" ? "" : spec.secs,
    trace: spec.trace,
    phase,
  };
}

function approval(partial: Partial<Approval> & Pick<Approval, "id" | "status">): Approval {
  const decided = partial.status !== "pending";
  return {
    id: partial.id,
    job_id: partial.job_id ?? "job_fixture",
    agent_id: partial.agent_id ?? "reviewer",
    agent_name: partial.agent_name ?? "Reviewer",
    thread_id: partial.thread_id ?? "thread_fixture",
    effect_class: partial.effect_class ?? "external",
    action: partial.action ?? "post_pr_comment",
    purpose:
      partial.purpose ??
      "Leave one review comment flagging an unhandled error path in the session handoff.",
    draft:
      partial.draft ??
      "If refresh() rejects on a 401, the handoff lock is never released. Wrap it in try/finally so the next session can take the lock.",
    evidence: partial.evidence ?? {
      repo: "DasVR/NIL",
      pr: "#212 handoff: release lock on refresh",
      ref: "phase0 @ a41c9e2",
      event_id: "ev_3f9a2c",
      kind: "tool.request",
    },
    evidence_text: partial.evidence_text ?? "repo DasVR/NIL",
    status: partial.status,
    provider: partial.provider ?? "mock",
    model: partial.model ?? "reviewer-small",
    usage_kind: partial.usage_kind ?? "estimated",
    input_tokens: partial.input_tokens ?? 2418,
    output_tokens: partial.output_tokens ?? 212,
    micro_usd: partial.micro_usd ?? 431,
    created_at: partial.created_at ?? Date.UTC(2026, 8, 29, 14, 42, 6),
    expires_at: partial.expires_at ?? null,
    decided_at: partial.decided_at ?? (decided ? Date.UTC(2026, 8, 29, 14, 43, 12) : null),
    decision_event_id: partial.decision_event_id ?? (decided ? "ev_4b1e07" : null),
    reason: partial.reason ?? null,
    committed: partial.committed ?? false,
    undo_until: partial.undo_until ?? null,
  };
}

function day(id: string, label: string): ThreadNode {
  return { type: "day", id, label };
}

function user(id: string, text: string, time: string): ThreadNode {
  return { type: "user", id, text, time, hlc: "" };
}

function bubblePart(id: string, text: string): TurnPart {
  return { type: "bubble", bubble: { id, text, shown: text } };
}

function toolsPart(block: ToolBlockView): TurnPart {
  return { type: "tools", block };
}

function turn(input: {
  id: string;
  agent: AgentKey;
  name: string;
  time: string;
  text?: string;
  parts?: TurnPart[];
  approval?: Approval | null;
}): ThreadNode {
  const parts = input.parts ?? (input.text ? [bubblePart(`${input.id}-b`, input.text)] : []);
  return {
    type: "turn",
    id: input.id,
    agent: input.agent,
    name: input.name,
    time: input.time,
    parts,
    approval: input.approval ?? null,
  };
}

function filedPush(): Approval {
  return approval({
    id: "ap_hist",
    status: "approved",
    action: "push_to_phase0",
    purpose: "Push the lock fix to phase0.",
    agent_name: "Builder",
    agent_id: "builder",
    committed: true,
    undo_until: null,
    decided_at: new Date(2026, 8, 28, 16, 31, 8).getTime(),
    decision_event_id: "ev_1b7e04",
    evidence: { repo: "DasVR/NIL", ref: "phase0", event_id: "ev_1a00aa", kind: "repo.push" },
  });
}

export function storyHistory(): ThreadNode[] {
  return [
    day("day-y", "Yesterday · 4:05 PM"),
    turn({
      id: "turn-y-r",
      agent: "reviewer",
      name: "Reviewer",
      time: "4:05 PM",
      text: "CI on #212 failed on phase0. Two sessions lost the handoff lock after `refresh()` returned a 401, and neither could take it back without a restart.",
    }),
    user("user-y", "Can someone take the refresh bug on #212? Sessions keep losing the handoff lock.", "4:12 PM"),
    turn({
      id: "turn-y-b",
      agent: "builder",
      name: "Builder",
      time: "4:13 PM",
      text: "On it. The lock is taken in `handoff.ts` and only released on the happy path. I’ll push a fix to phase0 and hand it to Reviewer.",
      approval: filedPush(),
    }),
    {
      type: "handoff",
      id: "hand-y",
      fromId: "ev_1b7e04",
      toId: "ev_1c2a9f",
      label: "Builder hands off to Reviewer",
      drawn: true,
    },
    day("day-t", "Today · 10:38 AM"),
    turn({
      id: "turn-t-r",
      agent: "reviewer",
      name: "Reviewer",
      time: "10:38 AM",
      text: "Woke on push `a41c9e2` to DasVR/NIL · phase0, “handoff: release lock on refresh”. Three files changed. Say the word and I’ll review it.",
    }),
    user("user-hold", "Hold on, pushing one more fix to the lock path first.", "10:39 AM"),
  ];
}

function foldedTools(open: boolean): ToolBlockView {
  return {
    id: "tools-main",
    folded: true,
    open,
    steps: STEPS.map((spec, index) => storyStep(spec, "done", index)),
  };
}

function reviewerAsk(card: Approval | null, waiting: ToolStepView["phase"]): ThreadNode {
  const post = storyStep(POST, waiting, 9);
  if (waiting === "waiting") {
    post.elapsed = "waiting on you";
  }
  if (waiting === "done") {
    post.elapsed = POST.secs;
  }
  if (card && card.status === "approved" && !card.committed) {
    post.elapsed = "approved · undo open";
    post.phase = "done";
    if (card.undo_until != null) {
      post.undo = { verb: "approved", until: card.undo_until };
    }
  }
  return turn({
    id: "turn-ask",
    agent: "reviewer",
    name: "Reviewer",
    time: "10:42 AM",
    parts: [
      toolsPart(foldedTools(false)),
      bubblePart("turn-ask-b", REPLY),
      toolsPart({ id: "tools-post", folded: false, open: false, steps: [post] }),
    ],
    approval: card,
  });
}

function pendingCard(undoUntil: number | null, committed: boolean): Approval {
  return approval({
    id: "ap_ask",
    status: committed ? "approved" : undoUntil ? "approved" : "pending",
    committed,
    undo_until: undoUntil,
    decision_event_id: undoUntil || committed ? "ev_4b1e07" : null,
    decided_at: undoUntil || committed ? Date.now() : null,
  });
}

export function frameForShot(shot: ShotName, now = Date.now()): ThreadFrame {
  const base = {
    draft: "",
    lit: false,
    sheenX: 360,
    cursor: null,
    chip: "Reviewer",
    placeholder: "Message Reviewer",
  };
  switch (shot) {
    case "send":
      return {
        ...base,
        nodes: [...storyHistory(), user("user-send", USER_SEND, "10:42 AM")],
      };
    case "tools":
      return {
        ...base,
        nodes: [
          ...storyHistory(),
          user("user-send", USER_SEND, "10:42 AM"),
          turn({
            id: "turn-tools",
            agent: "reviewer",
            name: "Reviewer",
            time: "10:42 AM",
            parts: [
              toolsPart({
                id: "tools-live",
                folded: false,
                open: false,
                steps: [
                  storyStep(STEPS[0], "done", 0),
                  storyStep(STEPS[1], "done", 1),
                  liveStep(STEPS[2], 2, "4.9s"),
                ],
              }),
            ],
          }),
        ],
      };
    case "approval":
      return {
        ...base,
        nodes: [
          ...storyHistory(),
          user("user-send", USER_SEND, "10:42 AM"),
          reviewerAsk(pendingCard(null, false), "waiting"),
        ],
      };
    case "receipt":
      return {
        ...base,
        nodes: [
          ...storyHistory(),
          user("user-send", USER_SEND, "10:42 AM"),
          reviewerAsk(pendingCard(null, true), "done"),
        ],
      };
    case "handoff":
      return {
        ...base,
        chip: "Builder",
        placeholder: "Message Builder",
        nodes: [
          ...storyHistory(),
          user("user-send", USER_SEND, "10:42 AM"),
          reviewerAsk(pendingCard(null, true), "done"),
          {
            type: "handoff",
            id: "hand-now",
            fromId: "ev_4b1e07",
            toId: "ev_4c83d5",
            label: "Reviewer hands off to Builder",
            drawn: true,
          },
          turn({
            id: "turn-builder",
            agent: "builder",
            name: "Builder",
            time: "10:42 AM",
            text: REPLY_BUILDER,
          }),
        ],
      };
    case "composer":
      return {
        ...base,
        lit: true,
        sheenX: 520,
        cursor: { x: 0, y: 0, visible: true },
        nodes: [
          ...storyHistory(),
          user("user-send", USER_SEND, "10:42 AM"),
          reviewerAsk(pendingCard(now + 6_000, false), "waiting"),
        ],
      };
    case "deny":
      return {
        ...base,
        nodes: [
          day("day-deny", "Today · 10:42 AM"),
          {
            type: "deny",
            id: "deny-shot",
            text: "Reviewer wanted to force-push phase0. Destructive actions are off in this build.",
            command: "git push --force origin phase0",
            time: "10:42 AM",
            hlc: "1759156920000:1:node",
            eventId: "ev_d3a91c",
          },
        ],
      };
    default: {
      const exhaustive: never = shot;
      return exhaustive;
    }
  }
}

export const STORY = {
  typed: USER_SEND,
  reply: REPLY,
  replyBuilder: REPLY_BUILDER,
  steps: STEPS,
  post: POST,
};

export function emptyFrame(): ThreadFrame {
  return {
    nodes: [],
    chip: "Reviewer",
    placeholder: "Message Reviewer",
    draft: "",
    lit: false,
    sheenX: 360,
    cursor: null,
  };
}
