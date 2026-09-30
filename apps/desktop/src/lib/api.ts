import { tokenMs } from "./cssTokens";

export type EffectClass = "read" | "write_local" | "external" | "destructive";
export type Decision = "approve" | "deny";
export type ApprovalStatus = "pending" | "approved" | "denied" | "expired";

/** User hold setting. The locked range is 400 to 1500. */
export const HOLD_MS_MIN = 400;
export const HOLD_MS_MAX = 1500;
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
  /** Mock `pr` row, e.g. `#212 handoff: release lock on refresh`. Empty when there is no PR. */
  pr?: string;
  /** Digits of the leading `#n` on `pr`. */
  pr_number?: string;
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
  payload: string;
}

/** Phase 1 C1. A destructive effect is this row, never a card. */
export interface DestructiveDenial {
  line: string;
  command: string;
}

export function destructiveDenial(payload: string): DestructiveDenial | null {
  let body: { policy?: string; line?: string; command?: string };
  try {
    body = JSON.parse(payload) as { policy?: string; line?: string; command?: string };
  } catch {
    return null;
  }
  if (body.policy !== "c1" || !body.line || !body.command) {
    return null;
  }
  return { line: body.line, command: body.command };
}

export interface Snapshot {
  protocol: number;
  role: string;
  node: string;
  provider: string;
  provider_detail: string;
  sync: string;
  endpoint_id: string | null;
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
      return "overwrites history. Can't be undone once it runs";
    default: {
      const exhaustive: never = effect;
      return exhaustive;
    }
  }
}

export function effectAsk(effect: EffectClass): string {
  switch (effect) {
    case "destructive":
      return "asks before overwriting";
    case "read":
    case "write_local":
    case "external":
      return "asks before posting";
    default: {
      const exhaustive: never = effect;
      return exhaustive;
    }
  }
}

const TITLE_ACRONYMS = new Set(["pr", "api", "url", "ui"]);

/** Sentence case for every action. `post_pr_comment` keeps its article. */
export function actionTitle(action: string): string {
  const source = action === "post_pr_comment" ? "post a pr comment" : action;
  return sentenceCase(source);
}

function sentenceCase(action: string): string {
  const words = action
    .replace(/[_-]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter((word) => word.length > 0);
  return words
    .map((word, index) => {
      const lower = word.toLowerCase();
      if (TITLE_ACRONYMS.has(lower)) {
        return lower.toUpperCase();
      }
      if (index === 0) {
        return lower.charAt(0).toUpperCase() + lower.slice(1);
      }
      return lower;
    })
    .join(" ");
}

export function clampHoldMs(ms: number): number {
  if (!Number.isFinite(ms)) {
    return tokenMs("--dur-hold", HOLD_MS_MIN);
  }
  return Math.min(HOLD_MS_MAX, Math.max(HOLD_MS_MIN, Math.round(ms)));
}

/** External hold. Destructive work has no card and no 1200ms hold (Phase 1 C1). */
export function holdDurationMs(_effect: EffectClass | null, setting?: number): number {
  if (setting == null) {
    return tokenMs("--dur-hold", HOLD_MS_MIN);
  }
  return clampHoldMs(setting);
}

/**
 * Phase 1 C4. Ask the platform authenticator, which is Windows Hello on Windows.
 * This does not draw a dialog. True means the OS verified the user. False means
 * they cancelled or the platform could not show Hello.
 */
export async function confirmWindowsHello(): Promise<boolean> {
  const credentialsApi = navigator.credentials;
  const platform = window.PublicKeyCredential;
  if (!credentialsApi?.create || !platform?.isUserVerifyingPlatformAuthenticatorAvailable) {
    return false;
  }
  const available = await platform.isUserVerifyingPlatformAuthenticatorAvailable();
  if (!available) {
    return false;
  }
  const challenge = crypto.getRandomValues(new Uint8Array(32));
  const userId = crypto.getRandomValues(new Uint8Array(16));
  try {
    const credential = await credentialsApi.create({
      publicKey: {
        challenge,
        rp: { name: "dasdevbot" },
        user: { id: userId, name: "dasdevbot", displayName: "dasdevbot" },
        pubKeyCredParams: [{ type: "public-key", alg: -7 }],
        timeout: 60_000,
        attestation: "none",
        authenticatorSelection: {
          authenticatorAttachment: "platform",
          userVerification: "required",
          residentKey: "discouraged",
        },
      },
    });
    return credential != null;
  } catch (error) {
    if (error instanceof DOMException) {
      return false;
    }
    return false;
  }
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

const streamClock = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
  hour12: true,
});

/** Local-zone clock time, e.g. `1:42 PM`. The zone is the runtime's, not a fixed offset. */
export function formatStreamTime(ms: number): string {
  return streamClock.format(ms);
}

/** Wall millis from an HLC stamp `millis:counter:node`, or null when it does not parse. */
export function hlcMillis(hlc: string): number | null {
  const head = hlc.split(":", 1)[0] ?? "";
  if (!/^\d+$/.test(head)) {
    return null;
  }
  const ms = Number(head);
  if (!Number.isSafeInteger(ms)) {
    return null;
  }
  return ms;
}

export function formatDecisionStamp(ms: number): string {
  const date = new Date(ms);
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  const ss = String(date.getSeconds()).padStart(2, "0");
  const zone =
    new Intl.DateTimeFormat("en-US", { timeZoneName: "short" })
      .formatToParts(date)
      .find((part) => part.type === "timeZoneName")?.value ?? "";
  return zone ? `${hh}:${mm}:${ss} ${zone}` : `${hh}:${mm}:${ss}`;
}

export function shortEventId(id: string): string {
  if (!id) {
    return "";
  }
  if (/^ev_/i.test(id)) {
    return id;
  }
  const compact = id.replace(/-/g, "");
  return `ev_${compact.slice(-6)}`;
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

/** Demo `repo.push`. `forced` asks for a destructive force-push instead of a PR comment. */
export async function emitPush(forced = false): Promise<void> {
  const response = await fetch("/v1/events", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      source: "demo",
      kind: "repo.push",
      payload: {
        repo: "DasVR/NIL",
        ref: forced ? "phase0" : "phase0 @ a41c9e2",
        pr: forced ? "" : "#212 handoff: release lock on refresh",
        purpose: forced
          ? ""
          : "Leave one review comment flagging an unhandled error path in the session handoff.",
        subject: "simulated push",
        note: "phase 0 attaches no diff",
        forced,
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
