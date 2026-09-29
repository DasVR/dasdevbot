export type EffectClass = "read" | "write_local" | "external" | "destructive";
export type Decision = "approve" | "deny";
export type ApprovalStatus = "pending" | "approved" | "denied" | "expired";

export const HOLD_MS_MIN = 400;
export const HOLD_MS_MAX = 1500;
export const HOLD_MS_DEFAULT = 600;
export const HOLD_MS_DESTRUCTIVE = 1200;
export const SEEN_LOCK_MS = 800;

export interface Agent {
  id: string;
  name: string;
  project: string;
  persona: string;
  token_cap: number;
  tokens_spent: number;
  status: string;
}

export interface Evidence {
  repo: string;
  ref: string;
  event_id: string;
  kind: string;
}

export interface Approval {
  id: string;
  job_id: string;
  agent_id: string;
  agent_name: string;
  thread_id: string;
  effect_class: string;
  action: string;
  purpose: string;
  draft: string;
  evidence: Evidence;
  evidence_text: string;
  status: string;
  provider: string;
  model: string;
  usage_kind: string;
  input_tokens: number;
  output_tokens: number;
  micro_usd: number;
  created_at: number;
  expires_at: number | null;
  decided_at: number | null;
  decision_event_id: string | null;
  reason: string | null;
  committed: boolean;
  undo_until: number | null;
}

export interface LedgerLine {
  id: string;
  agent_id: string;
  agent_name: string;
  project: string;
  provider: string;
  model: string;
  usage_kind: string;
  input_tokens: number;
  output_tokens: number;
  micro_usd: number;
  note: string;
}

export interface EventRow {
  id: string;
  version: number;
  hlc: string;
  source: string;
  kind: string;
  thread_id: string;
  idempotency_key: string;
}

export interface Snapshot {
  protocol: number;
  role: string;
  node: string;
  provider: string;
  provider_detail: string;
  sync: string;
  agents: Agent[];
  approvals: Approval[];
  ledger: LedgerLine[];
  events: EventRow[];
}

export function parseEffect(value: string): EffectClass | null {
  switch (value) {
    case "read":
    case "write_local":
    case "external":
    case "destructive":
      return value;
    default:
      return null;
  }
}

export function effectLabel(effect: EffectClass): string {
  switch (effect) {
    case "read":
      return "Read";
    case "write_local":
      return "Write local";
    case "external":
      return "External";
    case "destructive":
      return "Destructive";
    default: {
      const exhaustive: never = effect;
      return exhaustive;
    }
  }
}

export function effectWhy(effect: EffectClass): string {
  switch (effect) {
    case "read":
      return "";
    case "write_local":
      return "writes files on this machine";
    case "external":
      return "posts to GitHub, leaves this machine";
    case "destructive":
      return "removes or overwrites, and cannot be undone";
    default: {
      const exhaustive: never = effect;
      return exhaustive;
    }
  }
}

export function actionTitle(action: string): string {
  switch (action) {
    case "post_pr_comment":
      return "Post a PR comment";
    default:
      if (!action.includes("_") && action !== action.toLowerCase()) {
        return action;
      }
      return action
        .split("_")
        .filter((part) => part.length > 0)
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join(" ");
  }
}

export function clampHoldMs(ms: number): number {
  if (!Number.isFinite(ms)) {
    return HOLD_MS_DEFAULT;
  }
  return Math.min(HOLD_MS_MAX, Math.max(HOLD_MS_MIN, Math.round(ms)));
}

export function holdDurationMs(effect: EffectClass | null, setting: number): number {
  const base = clampHoldMs(setting);
  if (effect === "destructive") {
    return clampHoldMs(Math.max(base, HOLD_MS_DESTRUCTIVE));
  }
  return base;
}

export function formatUsd(micro: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 6,
  }).format(micro / 1_000_000);
}

export function formatTokens(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

export function formatDecisionStamp(ms: number): string {
  const date = new Date(ms);
  const time = new Intl.DateTimeFormat("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).format(date);
  const zone =
    new Intl.DateTimeFormat("en-US", { timeZoneName: "short" })
      .formatToParts(date)
      .find((part) => part.type === "timeZoneName")?.value ?? "";
  return zone ? `${time} ${zone}` : time;
}

export function shortEventId(id: string): string {
  if (!id) {
    return "";
  }
  if (/^ev_/i.test(id)) {
    return id;
  }
  const compact = id.replace(/-/g, "");
  return `ev_${compact.slice(-4)}`;
}

export function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
    return true;
  }
  if (target.isContentEditable) {
    return true;
  }
  return target.closest(".composer") !== null;
}

export async function getSnapshot(): Promise<Snapshot> {
  const response = await fetch("/v1/snapshot");
  if (!response.ok) {
    throw new Error(await readError(response));
  }
  return (await response.json()) as Snapshot;
}

export async function emitPush(): Promise<void> {
  const response = await fetch("/v1/events", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      source: "demo",
      kind: "repo.push",
      payload: {
        repo: "DasVR/NIL",
        ref: "phase0",
        subject: "simulated push",
        note: "phase 0 attaches no diff",
      },
      idempotency_key: `ui-${crypto.randomUUID()}`,
    }),
  });
  if (!response.ok) {
    throw new Error(await readError(response));
  }
}

export async function decide(id: string, decision: Decision, reason?: string): Promise<void> {
  const body: { decision: Decision; reason?: string } = { decision };
  const trimmed = reason?.trim();
  if (trimmed) {
    body.reason = trimmed;
  }
  const response = await fetch(`/v1/approvals/${id}/decision`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(await readError(response));
  }
}

export async function undo(id: string): Promise<void> {
  const response = await fetch(`/v1/approvals/${id}/undo`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: "{}",
  });
  if (!response.ok) {
    throw new Error(await readError(response));
  }
}

async function readError(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: string };
    return body.error ?? response.statusText;
  } catch {
    return response.statusText;
  }
}
