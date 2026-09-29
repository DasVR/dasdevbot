export type EffectClass = "read" | "write_local" | "external" | "destructive";
export type Decision = "approve" | "deny";

export interface Agent {
  id: string;
  name: string;
  project: string;
  persona: string;
  token_cap: number;
  tokens_spent: number;
  status: string;
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
  evidence: string;
  status: string;
  provider: string;
  model: string;
  usage_kind: string;
  input_tokens: number;
  output_tokens: number;
  micro_usd: number;
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

export function formatUsd(micro: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 6,
  }).format(micro / 1_000_000);
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

export async function decide(id: string, decision: Decision): Promise<void> {
  const response = await fetch(`/v1/approvals/${id}/decision`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ decision }),
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
