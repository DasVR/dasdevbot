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

/**
 * Every hold is --dur-hold. Destructive has no hold at all: it is denied by
 * policy and renders only as the flat ink row (C1), so there is no 1200ms path.
 */
export function holdDurationMs(_effect: EffectClass | null, setting?: number): number {
  return setting == null ? tokenMs("--dur-hold", HOLD_MS_MIN) : clampHoldMs(setting);
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

function shellToken(): string {
  const shell = globalThis as typeof globalThis & { __DASDEVBOT_TOKEN?: unknown };
  const value = shell.__DASDEVBOT_TOKEN;
  return typeof value === "string" ? value.trim() : "";
}

function metaToken(): string {
  if (typeof document === "undefined") {
    return "";
  }
  return (
    document.querySelector('meta[name="dasdevbot-token"]')?.getAttribute("content")?.trim() ?? ""
  );
}

function tauriInternals(): TauriInternals | null {
  const host = globalThis as typeof globalThis & { __TAURI_INTERNALS__?: TauriInternals };
  return host.__TAURI_INTERNALS__ ?? null;
}

async function sessionToken(): Promise<string> {
  const fromShell = shellToken();
  if (fromShell) {
    return fromShell;
  }
  const fromMeta = metaToken();
  if (fromMeta) {
    return fromMeta;
  }
  const invoke = tauriInternals()?.invoke;
  if (!invoke) {
    return "";
  }
  try {
    const value = await invoke("session_token");
    return typeof value === "string" ? value.trim() : "";
  } catch {
    return "";
  }
}

async function jsonHeaders(): Promise<Record<string, string>> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  const token = await sessionToken();
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

/** The Tauri window this page runs in, or null in a plain browser. */
export function tauriWindowLabel(): string | null {
  const host = globalThis as typeof globalThis & {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
  };
  return host.__TAURI_INTERNALS__?.metadata?.currentWindow?.label ?? null;
}

/** Show the card window. Only the card window can sign or undo a decision. */
export async function openCardWindow(): Promise<void> {
  await tauriInvoke()("open_card_window");
}

export async function getSnapshot(): Promise<Snapshot> {
  // A bundled webview is on the Tauri origin and the daemon sends no CORS
  // headers, so the shell reads the snapshot over loopback for it.
  if (tauriInternals()) {
    return (await tauriInvoke()("daemon_snapshot")) as Snapshot;
  }
  const response = await fetch("/v1/snapshot");
  if (!response.ok) {
    throw new Error(await readError(response));
  }
  return (await response.json()) as Snapshot;
}

/** Demo `repo.push`. The demo never asks for a destructive force-push. */
export async function emitPush(): Promise<void> {
  if (tauriInternals()) {
    await tauriInvoke()("daemon_emit_demo", { forced: false });
    return;
  }
  const response = await fetch("/v1/events", {
    method: "POST",
    headers: await jsonHeaders(),
    body: JSON.stringify({
      source: "demo",
      kind: "repo.push",
      payload: {
        repo: "DasVR/NIL",
        ref: "phase0",
        subject: "simulated push",
        note: "phase 0 attaches no diff",
        forced: false,
      },
      idempotency_key: `ui-${crypto.randomUUID()}`,
    }),
  });
  if (!response.ok) {
    throw new Error(await readError(response));
  }
}

type TauriInternals = {
  invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
};

function tauriInvoke(): NonNullable<TauriInternals["invoke"]> {
  const internals = (globalThis as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__;
  if (!internals?.invoke) {
    throw new Error("approval decisions are Tauri IPC only");
  }
  return internals.invoke.bind(internals);
}

/** The OS Hello prompt was cancelled or refused. The card goes back to waiting quietly. */
export function isHelloCancel(message: string): boolean {
  return /hello consent was denied|cancel/i.test(message);
}

export async function decide(id: string, decision: Decision, reason?: string): Promise<void> {
  const trimmed = reason?.trim();
  await tauriInvoke()("sign_decision", {
    id,
    decision,
    reason: trimmed ? trimmed : null,
  });
}

export async function undo(id: string): Promise<void> {
  await tauriInvoke()("undo_decision", { id });
}

export async function setSecret(handle: string, value: string): Promise<{ last4: string }> {
  const result = await tauriInvoke()("set_secret", { handle, value });
  if (!result || typeof result !== "object" || !("last4" in result)) {
    throw new Error("secret entry failed");
  }
  const last4 = (result as { last4: unknown }).last4;
  if (typeof last4 !== "string") {
    throw new Error("secret entry failed");
  }
  return { last4 };
}

async function readError(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as { error?: string };
    return body.error ?? response.statusText;
  } catch {
    return response.statusText;
  }
}
