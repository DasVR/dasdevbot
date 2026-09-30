/** First-run progress. Quitting mid-flow resumes at the same step. */

export const FIRST_RUN_KEY = "dasdevbot.first-run";
export const DEFAULT_ADDRESS = "127.0.0.1:7421";

export type FirstRunStep = "helper" | "github" | "repo" | "rules";
export type GithubPhase = "idle" | "waiting" | "connected";
export type RepoAccess = "ok" | "missing";

export interface RepoChoice {
  fullName: string;
  defaultBranch: string;
  access: RepoAccess;
}

export interface FirstRunRecord {
  version: 1;
  completed: boolean;
  step: FirstRunStep;
  helperSkipped: boolean;
  github: GithubPhase;
  handoffUrl: string | null;
  login: string | null;
  repo: string | null;
  address: string;
  elsewhere: boolean;
  askPost: boolean;
  askWrite: boolean;
}

export interface TextPart {
  text: string;
  match: boolean;
}

/** Fixture catalog. A real GitHub session replaces this. It carries no token. */
export const DEV_REPOS: readonly RepoChoice[] = [
  { fullName: "DasVR/NIL", defaultBranch: "main", access: "ok" },
  { fullName: "DasVR/dasdevbot", defaultBranch: "main", access: "ok" },
  { fullName: "DasVR/ledger-notes", defaultBranch: "main", access: "missing" },
];

const STEPS: readonly FirstRunStep[] = ["helper", "github", "repo", "rules"];

export function emptyRecord(): FirstRunRecord {
  return {
    version: 1,
    completed: false,
    step: "helper",
    helperSkipped: false,
    github: "idle",
    handoffUrl: null,
    login: null,
    repo: null,
    address: DEFAULT_ADDRESS,
    elsewhere: false,
    askPost: true,
    askWrite: true,
  };
}

export function isFirstRunComplete(): boolean {
  return loadRecord()?.completed === true;
}

export function loadRecord(): FirstRunRecord | null {
  try {
    const raw = localStorage.getItem(FIRST_RUN_KEY);
    if (!raw) {
      return null;
    }
    return parseRecord(JSON.parse(raw) as unknown, window.location.origin);
  } catch {
    return null;
  }
}

export function saveRecord(record: FirstRunRecord): void {
  try {
    localStorage.setItem(FIRST_RUN_KEY, JSON.stringify(record));
  } catch {
    // Private-mode storage can refuse the write. The in-memory step still moves.
  }
}

export function parseRecord(value: unknown, origin: string): FirstRunRecord | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const row = value as Record<string, unknown>;
  if (row.version !== 1) {
    return null;
  }
  if (row.completed === true) {
    const done = emptyRecord();
    done.completed = true;
    done.step = "rules";
    done.helperSkipped = true;
    done.github = "connected";
    done.login = loginName(row.login);
    done.repo = repoName(row.repo) ?? "DasVR/NIL";
    done.address =
      typeof row.address === "string" && normalizeAddress(row.address) ? row.address : DEFAULT_ADDRESS;
    return done;
  }
  if (!isStep(row.step) || !isGithub(row.github)) {
    return null;
  }
  const address = typeof row.address === "string" && normalizeAddress(row.address) ? row.address : DEFAULT_ADDRESS;
  return {
    version: 1,
    completed: false,
    step: row.step,
    helperSkipped: row.helperSkipped === true,
    github: row.github,
    handoffUrl: safeHandoff(row.handoffUrl, origin),
    login: loginName(row.login),
    repo: repoName(row.repo),
    address,
    elsewhere: row.elsewhere === true,
    askPost: row.askPost !== false,
    askWrite: row.askWrite !== false,
  };
}

export function stepNumber(step: FirstRunStep): 1 | 2 | 3 | 4 {
  switch (step) {
    case "helper":
      return 1;
    case "github":
      return 2;
    case "repo":
      return 3;
    case "rules":
      return 4;
    default: {
      const exhaustive: never = step;
      return exhaustive;
    }
  }
}

export function stepTitle(step: FirstRunStep): string {
  switch (step) {
    case "helper":
      return "dasdevbot runs a small helper on this machine.";
    case "github":
      return "Connect GitHub.";
    case "repo":
      return "Pick a repo.";
    case "rules":
      return "Everyone asks before it acts.";
    default: {
      const exhaustive: never = step;
      return exhaustive;
    }
  }
}

export function previousStep(step: FirstRunStep, helperSkipped: boolean): FirstRunStep | null {
  switch (step) {
    case "helper":
      return null;
    case "github":
      return helperSkipped ? null : "helper";
    case "repo":
      return "github";
    case "rules":
      return "repo";
    default: {
      const exhaustive: never = step;
      return exhaustive;
    }
  }
}

export function normalizeAddress(value: string): string | null {
  const trimmed = value.trim();
  const match = /^([A-Za-z0-9.-]+):(\d{1,5})$/.exec(trimmed);
  if (!match) {
    return null;
  }
  const port = Number(match[2]);
  if (port < 1 || port > 65535) {
    return null;
  }
  return `${match[1]}:${String(port)}`;
}

export function visibleRepos(repos: readonly RepoChoice[], query: string): readonly RepoChoice[] {
  const needle = query.trim().toLowerCase();
  if (!needle) {
    return repos;
  }
  return repos.filter((repo) => repo.fullName.toLowerCase().includes(needle));
}

export function canPick(repo: RepoChoice): boolean {
  switch (repo.access) {
    case "ok":
      return true;
    case "missing":
      return false;
    default: {
      const exhaustive: never = repo.access;
      return exhaustive;
    }
  }
}

export function highlightParts(name: string, query: string): readonly TextPart[] {
  const needle = query.trim().toLowerCase();
  if (!needle) {
    return [{ text: name, match: false }];
  }
  const index = name.toLowerCase().indexOf(needle);
  if (index < 0) {
    return [{ text: name, match: false }];
  }
  const parts = [
    { text: name.slice(0, index), match: false },
    { text: name.slice(index, index + needle.length), match: true },
    { text: name.slice(index + needle.length), match: false },
  ];
  return parts.filter((part) => part.text.length > 0);
}

export function emptyRepoCopy(query: string): string {
  return `No repo matches “${query.trim()}”.`;
}

export function fixAccessHref(fullName: string): string {
  return `https://github.com/${fullName}/settings/access`;
}

export function helperMissCopy(address: string): string {
  return `The helper didn't answer at ${address}. Nothing is running yet.`;
}

export const GITHUB_LEAD = "Reviewer reads your pushes there. It asks before it posts anything.";
export const REPO_LEAD = "Reviewer watches one repo in this build.";

export function connectedCopy(login: string): string {
  return `Connected as ${login}`;
}

export function helperSkipCopy(address: string): string {
  return `Helper found on this machine, step 1 skipped · ${address}`;
}

export function landingCopy(repo: string): string {
  return `Reviewer reads every push to ${repo} and asks before it posts to GitHub.`;
}

/** GitHub login from the handoff. Rejects anything that is not a login. */
export function loginName(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  if (!/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$/.test(value)) {
    return null;
  }
  return value;
}

function isStep(value: unknown): value is FirstRunStep {
  return typeof value === "string" && (STEPS as readonly string[]).includes(value);
}

function isGithub(value: unknown): value is GithubPhase {
  return value === "idle" || value === "waiting" || value === "connected";
}

function repoName(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) {
    return null;
  }
  return value;
}

function safeHandoff(value: unknown, origin: string): string | null {
  if (typeof value !== "string" || origin === "") {
    return null;
  }
  try {
    const url = new URL(value);
    if (url.origin !== origin || url.pathname !== "/github-device.html") {
      return null;
    }
    return url.toString();
  } catch {
    return null;
  }
}
