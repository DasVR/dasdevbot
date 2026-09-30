import { DEV_REPOS, loginName, type RepoChoice } from "./model";

/**
 * Seams for the machine around the first-run screens.
 * The GitHub step is a dev handoff: same shape as a device flow, no client id,
 * no access token, and no embedded webview. A production connector replaces
 * `githubHandoffUrl` and still calls `SystemBrowser.openExternal`.
 */

export const HANDOFF_CHANNEL = "dasdevbot-github-handoff";
export const HANDOFF_STORAGE_KEY = "dasdevbot.github-handoff";

export type HandoffOutcome = "connected" | "cancelled";

export interface HandoffPayload {
  type: HandoffOutcome;
  login: string | null;
}
export type HelperStart = "started" | "failed";

export interface SystemBrowser {
  /** Hand the URL to the OS default browser. Never an embedded webview. */
  openExternal(url: string): void;
}

export interface HelperPort {
  /** Silent probe. The screen draws nothing while this is in flight. */
  find(address: string): Promise<boolean>;
  start(address: string): Promise<HelperStart>;
}

export interface NotificationPort {
  request(): Promise<NotificationPermission | "unsupported">;
}

export interface MicPort {
  request(): Promise<void>;
}

export interface HandoffListen {
  done: Promise<HandoffPayload>;
  stop(): void;
}

interface HandoffMessage {
  v: 1;
  type: HandoffOutcome;
  login: string | null;
}

declare global {
  interface Window {
    __dasdevbot?: {
      browser?: SystemBrowser;
      helper?: HelperPort;
      notifications?: NotificationPort;
      mic?: MicPort;
      repos?: readonly RepoChoice[];
    };
  }
}

const nativeBrowser: SystemBrowser = {
  openExternal(url: string): void {
    window.open(url, "_blank");
  },
};

export function browserPort(): SystemBrowser {
  return window.__dasdevbot?.browser ?? nativeBrowser;
}

export function helperPort(): HelperPort {
  return window.__dasdevbot?.helper ?? loopbackHelper;
}

export function notificationPort(): NotificationPort {
  return window.__dasdevbot?.notifications ?? systemNotifications;
}

export function micPort(): MicPort {
  return window.__dasdevbot?.mic ?? systemMic;
}

export function repoCatalog(): readonly RepoChoice[] {
  return window.__dasdevbot?.repos ?? DEV_REPOS;
}

const loopbackHelper: HelperPort = {
  find(address: string): Promise<boolean> {
    return probeLoopback(address);
  },
  async start(address: string): Promise<HelperStart> {
    const found = await probeLoopback(address);
    return found ? "started" : "failed";
  },
};

export async function probeLoopback(address: string): Promise<boolean> {
  const timeout = new Promise<never>((_, reject) => {
    setTimeout(() => reject(new Error("timeout")), 700);
  });
  try {
    await Promise.race([
      fetch(`http://${address}/v1/health`, { mode: "no-cors" }),
      timeout,
    ]);
    return true;
  } catch {
    return false;
  }
}

const systemNotifications: NotificationPort = {
  async request(): Promise<NotificationPermission | "unsupported"> {
    if (typeof Notification === "undefined" || typeof Notification.requestPermission !== "function") {
      return "unsupported";
    }
    const result = await Notification.requestPermission();
    switch (result) {
      case "granted":
      case "denied":
      case "default":
        return result;
      default: {
        const exhaustive: never = result;
        return exhaustive;
      }
    }
  },
};

const systemMic: MicPort = {
  async request(): Promise<void> {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    for (const track of stream.getTracks()) {
      track.stop();
    }
  },
};

export function devicePageUrl(): string {
  const url = new URL("/github-device.html", window.location.href);
  url.searchParams.set("handoff", "dev");
  return url.toString();
}

export function postHandoff(outcome: HandoffOutcome, login?: string): void {
  const message: HandoffMessage = { v: 1, type: outcome, login: loginName(login) };
  const channel = new BroadcastChannel(HANDOFF_CHANNEL);
  channel.postMessage(message);
  channel.close();
  localStorage.setItem(HANDOFF_STORAGE_KEY, JSON.stringify({ ...message, at: Date.now() }));
}

export function consumeHandoff(): HandoffPayload | null {
  try {
    const raw = localStorage.getItem(HANDOFF_STORAGE_KEY);
    if (!raw) {
      return null;
    }
    localStorage.removeItem(HANDOFF_STORAGE_KEY);
    return payloadOf(JSON.parse(raw) as unknown);
  } catch {
    return null;
  }
}

export function listenForHandoff(): HandoffListen {
  let settled = false;
  let resolveDone: (outcome: HandoffPayload) => void = () => {};
  const done = new Promise<HandoffPayload>((resolve) => {
    resolveDone = resolve;
  });
  const channel = new BroadcastChannel(HANDOFF_CHANNEL);
  const finish = (outcome: HandoffPayload) => {
    if (settled) {
      return;
    }
    settled = true;
    channel.close();
    window.removeEventListener("storage", onStorage);
    resolveDone(outcome);
  };
  const onStorage = (event: StorageEvent) => {
    if (event.key !== HANDOFF_STORAGE_KEY || !event.newValue) {
      return;
    }
    const outcome = payloadOf(safeParse(event.newValue));
    if (outcome) {
      finish(outcome);
    }
  };
  channel.onmessage = (event: MessageEvent<unknown>) => {
    const outcome = payloadOf(event.data);
    if (outcome) {
      finish(outcome);
    }
  };
  window.addEventListener("storage", onStorage);
  return {
    done,
    stop() {
      if (settled) {
        return;
      }
      settled = true;
      channel.close();
      window.removeEventListener("storage", onStorage);
    },
  };
}

function payloadOf(value: unknown): HandoffPayload | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const type = (value as { type?: unknown }).type;
  if (type !== "connected" && type !== "cancelled") {
    return null;
  }
  const login = type === "connected" ? loginName((value as { login?: unknown }).login) : null;
  return { type, login };
}

function safeParse(raw: string): unknown {
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return null;
  }
}
