/** Short non-secret job id plus elapsed time. Lease tokens and epochs are never rendered. */
export function leaseLine(id: string, elapsed: string): string {
  return `lease ${id} · ${elapsed}`;
}

/** Reviewer's job id. The roster-width example is `lease rev_01 · 2m14s`. */
export const REVIEWER_LEASE_ID = "rev_01";

const LEASE_IDS: Record<string, string> = {
  reviewer: REVIEWER_LEASE_ID,
  builder: "bld_02",
  deployer: "dep_03",
};

export function leaseIdFor(agentId: string): string {
  return LEASE_IDS[agentId] ?? agentId;
}

export type RosterKind = "quiet" | "waiting" | "lease" | "note";

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
}

export interface ReviewerChrome {
  waiting: boolean;
  filing: string | null;
  working: boolean;
}

/** The three teammates on the shell mock. The status line is a job id, not a lease token. */
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
      kind: "lease",
      line: leaseLine("bld_02", "2m14s"),
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
        kind: "lease" as const,
        line: leaseLine(leaseIdFor(agent.id), "0m00s"),
      };
    }
    return { ...base, kind: "quiet" as const, line: "" };
  });
}
