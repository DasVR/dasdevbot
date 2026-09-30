import { actionTitle, formatStreamTime, parseEffect, type Approval, type EffectClass } from "./api";

/** A row within this window keeps its place and a frozen "expires m:ss" label. */
export const EXPIRING_WINDOW_MS = 2 * 60 * 1000;

export function byOldest(a: Approval, b: Approval): number {
  if (a.created_at !== b.created_at) {
    return a.created_at - b.created_at;
  }
  if (a.id < b.id) {
    return -1;
  }
  if (a.id > b.id) {
    return 1;
  }
  return 0;
}

export function isDestructive(approval: Approval): boolean {
  return parseEffect(approval.effect_class) === "destructive";
}

export function remainingMs(approval: Approval, nowMs: number): number | null {
  if (approval.expires_at == null) {
    return null;
  }
  return approval.expires_at - nowMs;
}

/** Pending, not destructive, and not past expires_at. Oldest first. */
export function waitingApprovals(approvals: Approval[], nowMs: number): Approval[] {
  return approvals
    .filter((approval) => {
      if (approval.status !== "pending" || isDestructive(approval)) {
        return false;
      }
      const remaining = remainingMs(approval, nowMs);
      return remaining == null || remaining > 0;
    })
    .sort(byOldest);
}

export function destructiveApprovals(approvals: Approval[]): Approval[] {
  return approvals.filter((approval) => approval.status === "pending" && isDestructive(approval));
}

/** Static label. Callers freeze the first value; this does not tick. */
export function formatExpires(remaining: number): string {
  const total = Math.max(0, Math.ceil(remaining / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `expires ${minutes}:${String(seconds).padStart(2, "0")}`;
}

export function isExpiringSoon(approval: Approval, nowMs: number): boolean {
  const remaining = remainingMs(approval, nowMs);
  return remaining != null && remaining > 0 && remaining <= EXPIRING_WINDOW_MS;
}

function riskWord(effect: EffectClass | null): string {
  switch (effect) {
    case "read":
      return "read";
    case "write_local":
      return "write local";
    case "external":
      return "external";
    case "destructive":
      return "destructive";
    case null:
      return "unknown";
    default: {
      const exhaustive: never = effect;
      return exhaustive;
    }
  }
}

/** Right-hand mono. An expiring row replaces it with the frozen expires label. */
export function queueMeta(approval: Approval, expiresLabel: string | null): string {
  if (expiresLabel) {
    return expiresLabel;
  }
  const effect = parseEffect(approval.effect_class);
  const ref = approval.evidence.ref;
  const target = ref ? `${approval.evidence.repo} · ${ref}` : approval.evidence.repo;
  return `${target} · ${riskWord(effect)} · ${formatStreamTime(approval.created_at)}`;
}

export function queueTitle(approval: Approval): string {
  return actionTitle(approval.action);
}

/** Second clause is only for a mode return, which this build does not open. */
export function queueSummary(waiting: number): string {
  return `${waiting} waiting on you`;
}

export interface SheetRow {
  id: string;
  agentId: string;
  agent: string;
  title: string;
  meta: string;
  expiring: boolean;
  expired: boolean;
  createdAt: number;
}

export function toSheetRow(
  approval: Approval,
  frozen: string | null,
  expired: boolean,
): SheetRow {
  return {
    id: approval.id,
    agentId: approval.agent_id,
    agent: approval.agent_name,
    title: expired
      ? "Expired · Reviewer will ask again on the next push."
      : queueTitle(approval),
    meta: expired ? "" : queueMeta(approval, frozen),
    expiring: !expired && frozen != null,
    expired,
    createdAt: approval.created_at,
  };
}
