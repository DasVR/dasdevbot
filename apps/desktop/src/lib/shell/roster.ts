/**
 * A running teammate's sub, in the mock's two-line shape
 * ("… · 2m14s · 208 / 8000 tok", CD ruling 2). The mock's lease label is
 * replaced by the state word: no lease id, token or fencing epoch reaches the DOM.
 */
export function runningLine(elapsed: string | null, spent?: number | null, cap?: number | null): string {
  const parts = ["running"];
  if (elapsed) {
    parts.push(elapsed);
  }
  if (spent != null && cap != null && cap > 0) {
    parts.push(`${spent} / ${cap} tok`);
  }
  return parts.join(" · ");
}

/** Elapsed time in the mock's `2m14s` shape. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}m${String(seconds).padStart(2, "0")}s`;
}

export type RosterKind = "quiet" | "waiting" | "running" | "note";

export interface RosterRow {
  id: string;
  name: string;
  monogram: string;
  kind: RosterKind;
  line: string;
  selected: boolean;
}

export interface LiveAgent {
  id: string;
  name: string;
  status: string;
  /** How long this window has seen the teammate running, or null. It ticks with the app clock. */
  runningMs?: number | null;
  tokensSpent?: number | null;
  tokenCap?: number | null;
}

export interface ReviewerChrome {
  waiting: boolean;
  filing: string | null;
  working: boolean;
}

/** The three teammates on the shell mock. Builder's mock lease label is replaced by its state. */
export function stageRoster(): RosterRow[] {
  return [
    {
      id: "reviewer",
      name: "Reviewer",
      monogram: "R",
      kind: "waiting",
      line: "waiting on you",
      selected: true,
    },
    {
      id: "builder",
      name: "Builder",
      monogram: "B",
      kind: "running",
      line: runningLine("2m14s", 208, 8000),
      selected: false,
    },
    {
      id: "deployer",
      name: "Deployer",
      monogram: "D",
      kind: "note",
      line: "done · 10:58 AM",
      selected: false,
    },
  ];
}

export function liveRoster(
  agents: LiveAgent[],
  reviewer: ReviewerChrome | null,
  selectedId: string,
): RosterRow[] {
  return agents.map((agent) => {
    const selected = agent.id === selectedId;
    const monogram = agent.name.slice(0, 1).toUpperCase() || "·";
    const base = { id: agent.id, name: agent.name, monogram, selected };
    if (agent.id === "reviewer" && reviewer?.waiting) {
      return { ...base, kind: "waiting" as const, line: "waiting on you" };
    }
    if (agent.id === "reviewer" && reviewer?.filing) {
      return { ...base, kind: "note" as const, line: reviewer.filing };
    }
    if (agent.status === "working" || (agent.id === "reviewer" && reviewer?.working)) {
      return {
        ...base,
        kind: "running" as const,
        line: runningLine(
          agent.runningMs == null ? null : formatElapsed(agent.runningMs),
          agent.tokensSpent,
          agent.tokenCap,
        ),
      };
    }
    return { ...base, kind: "quiet" as const, line: "" };
  });
}
