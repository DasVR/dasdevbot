import type { Approval } from "../api";

export type AgentKey = "reviewer" | "builder";

export type TraceKind = "reading" | "writing";

export type StepPhase = "live" | "done" | "waiting";

export interface ToolStepView {
  id: string;
  say: string;
  detail: string;
  elapsed: string;
  trace: TraceKind;
  phase: StepPhase;
  /** Open undo window on the post step. The row counts `<verb> · undo Ns` down to it. */
  undo?: { verb: "approved" | "denied"; until: number } | null;
}

export interface ToolBlockView {
  id: string;
  steps: ToolStepView[];
  folded: boolean;
  open: boolean;
}

export interface BubbleView {
  id: string;
  /** Full reply. Streaming reveals `shown`. */
  text: string;
  shown: string;
}

/** One turn renders these in order. The prototype's ask is tools, reply, post step, then the slot. */
export type TurnPart = { type: "bubble"; bubble: BubbleView } | { type: "tools"; block: ToolBlockView };

export interface CursorMark {
  x: number;
  y: number;
  visible: boolean;
}

export type ThreadNode =
  | { type: "day"; id: string; label: string }
  | { type: "user"; id: string; text: string; time: string; hlc: string }
  | {
      type: "turn";
      id: string;
      agent: AgentKey;
      name: string;
      time: string;
      parts: TurnPart[];
      /** Card or receipt in this turn's gutter slot. Null when the turn is only talk. */
      approval: Approval | null;
    }
  | {
      type: "handoff";
      id: string;
      fromId: string;
      toId: string;
      label: string;
      drawn: boolean;
    }
  | { type: "system"; id: string; text: string; time: string; hlc: string; eventId: string }
  | { type: "deny"; id: string; text: string; command: string; time: string; hlc: string; eventId: string };

export interface ThreadFrame {
  nodes: ThreadNode[];
  chip: string;
  placeholder: string;
  draft: string;
  lit: boolean;
  /** Sheen position inside the composer, in px. Only used while `lit`. */
  sheenX: number;
  cursor: CursorMark | null;
}
