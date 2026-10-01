import {
  actionTitle,
  formatStreamTime,
  hlcMillis,
  shortEventId,
  type Approval,
  type EventRow,
  type Snapshot,
} from "../api";
import type { ThreadNode } from "./types";

const APPROVAL_KINDS = new Set([
  "approval.requested",
  "approval.decided",
  "approval.undone",
  "approval.committed",
  "approval.expired",
]);

export interface LocalNote {
  id: string;
  text: string;
  at: number;
}

/** Phase 1: a destructive ask is a flat row, never a card. */
export function destructiveCopy(approval: Approval): { text: string; command: string } {
  const name = approval.agent_name.trim() || "The teammate";
  const ref = approval.evidence.ref.trim();
  const verb =
    approval.action === "force_push"
      ? `force-push ${ref || "the branch"}`
      : lowerFirst(actionTitle(approval.action));
  return {
    text: `${name} wanted to ${verb}. Destructive actions are off in this build.`,
    command: approval.draft.trim(),
  };
}

function lowerFirst(value: string): string {
  if (!value) {
    return "run that";
  }
  return value.charAt(0).toLowerCase() + value.slice(1);
}

function systemText(kind: string): string {
  switch (kind) {
    case "repo.push":
      return "Push received";
    case "ledger.posted":
      return "Spend recorded";
    case "budget.denied":
      return "Budget held this turn. Nothing ran.";
    case "job.failed":
      return "The turn stopped. Nothing was posted.";
    case "session.note":
    case "note":
    case "Note":
      return "Note recorded";
    case "draft.saved":
      return "Draft saved";
    default: {
      const words = kind.replace(/[._]+/g, " ").trim();
      if (!words) {
        return "Recorded";
      }
      return words.charAt(0).toUpperCase() + words.slice(1);
    }
  }
}

function eventTime(event: EventRow): string {
  const millis = hlcMillis(event.hlc);
  return millis == null ? "" : formatStreamTime(millis);
}

/**
 * Daemon events, oldest first, as thread rows.
 * Approval kinds are not rows: the card or the receipt owns that slot, and a
 * filed approval never turns back into the request that produced it.
 */
export function threadFromSnapshot(snapshot: Snapshot, notes: LocalNote[]): ThreadNode[] {
  const nodes: ThreadNode[] = [];
  const events = [...snapshot.events].reverse();
  const placed = new Set<string>();

  for (const event of events) {
    if (APPROVAL_KINDS.has(event.kind)) {
      const approval = approvalForEvent(snapshot.approvals, event);
      if (!approval || placed.has(approval.id)) {
        continue;
      }
      placed.add(approval.id);
      nodes.push(approvalNode(approval, event));
      continue;
    }
    nodes.push({
      type: "system",
      id: event.id,
      text: systemText(event.kind),
      time: eventTime(event),
      hlc: event.hlc,
      eventId: shortEventId(event.id),
    });
  }

  for (const approval of snapshot.approvals) {
    if (placed.has(approval.id)) {
      continue;
    }
    placed.add(approval.id);
    nodes.push(approvalNode(approval, null));
  }

  const reviewer = snapshot.agents.find((agent) => agent.id === "reviewer");
  const waiting = snapshot.approvals.some((approval) => approval.status === "pending");
  if (reviewer?.status === "working" && !waiting) {
    nodes.push({
      type: "turn",
      id: "turn-working",
      agent: "reviewer",
      name: reviewer.name,
      time: "",
      parts: [
        {
          type: "tools",
          block: {
            id: "tools-working",
            folded: false,
            open: false,
            steps: [
              {
                id: "step-working",
                say: `${reviewer.name} is working`,
                detail: `${reviewer.tokens_spent} / ${reviewer.token_cap} tok`,
                elapsed: "",
                trace: "reading",
                phase: "live",
              },
            ],
          },
        },
      ],
      approval: null,
    });
  }

  for (const note of notes) {
    nodes.push({
      type: "user",
      id: note.id,
      text: note.text,
      time: formatStreamTime(note.at),
      hlc: "",
    });
  }

  return nodes;
}

function approvalForEvent(approvals: Approval[], event: EventRow): Approval | null {
  const fromKey = event.idempotency_key.startsWith("approval-requested:")
    ? event.idempotency_key.slice("approval-requested:".length)
    : "";
  if (fromKey) {
    return approvals.find((approval) => approval.id === fromKey) ?? null;
  }
  return (
    approvals.find((approval) => approval.decision_event_id === event.id) ??
    approvals.find((approval) => approval.evidence.event_id === event.id) ??
    null
  );
}

function approvalNode(approval: Approval, event: EventRow | null): ThreadNode {
  if (approval.effect_class === "destructive") {
    const copy = destructiveCopy(approval);
    const millis = approval.decided_at ?? approval.created_at;
    return {
      type: "deny",
      id: `deny-${approval.id}`,
      text: copy.text,
      command: copy.command,
      time: formatStreamTime(millis),
      hlc: event?.hlc ?? "",
      eventId: shortEventId(approval.decision_event_id || approval.evidence.event_id || approval.id),
    };
  }
  return {
    type: "turn",
    id: `turn-${approval.id}`,
    agent: "reviewer",
    name: approval.agent_name || "Reviewer",
    time: formatStreamTime(approval.created_at),
    parts: [],
    approval,
  };
}
