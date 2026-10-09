/**
 * A running teammate's sub: the video's "lease bld_02 · 2m14s · 208 / 8000 tok"
 * without the lease segment, so "2m14s · 208 / 8000 tok" (DE ruling 2, CD
 * ruling b). Lease identifiers never reach the DOM (UID #24 item 7); that is a
 * declared deviation from the video frame. With no live values it says "running".
 */
export function runningLine(elapsed: string | null, spent?: number | null, cap?: number | null): string {
  const parts: string[] = [];
  if (elapsed) {
    parts.push(elapsed);
  }
  if (spent != null && cap != null && cap > 0) {
    parts.push(`${spent} / ${cap} tok`);
  }
  return parts.length > 0 ? parts.join(" · ") : "running";
}

/**
 * CD ruling b: the video wraps Builder's sub as "lease bld_02 · 2m14s · 208 /"
 * then "8000 tok". Without the lease segment the line would fit on one line,
 * so the break is placed where the video has it: before the cap. Returns the
 * two lines, or null when the line has no "spent / cap tok" part.
 * textContent stays "2m14s · 208 / 8000 tok".
 */
export function videoWrap(line: string): [string, string] | null {
  const match = /^(.* \/) (\d+ tok)$/.exec(line);
  return match ? [match[1], match[2]] : null;
}

/** Elapsed time in the mock's `2m14s` shape. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}m${String(seconds).padStart(2, "0")}s`;
}

/** "asking": waiting on you, without the dot (the mock keeps the dot on the selected row). */
export type RosterKind = "quiet" | "waiting" | "asking" | "running" | "note";

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

/**
 * The three teammates on the shell mock. Builder's mock lease label is
 * replaced by its state. When the scripted count bumps to 2, Builder asks too:
 * its sub becomes "waiting on you" on one line, on the same tick (mock
 * 4a:1324, video frame 282). The dot stays on the selected teammate only.
 */
export function stageRoster(builderAsks = false): RosterRow[] {
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
      kind: builderAsks ? "asking" : "running",
      line: builderAsks ? "waiting on you" : runningLine("2m14s", 208, 8000),
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
