/** Permission asks. One ink line is on screen before the OS dialog. */

export const PERMISSIONS_KEY = "dasdevbot.permissions";

export type PermissionKind = "notifications" | "mic";

export interface PermissionMemory {
  notifications: boolean;
  mic: boolean;
}

export interface AskAfterLine {
  line: string;
  show(line: string): void | Promise<void>;
  hide(): void;
  paint(): Promise<void>;
  request(): Promise<unknown>;
}

export function loadPermissionMemory(): PermissionMemory {
  try {
    const raw = localStorage.getItem(PERMISSIONS_KEY);
    if (!raw) {
      return { notifications: false, mic: false };
    }
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") {
      return { notifications: false, mic: false };
    }
    const row = parsed as Record<string, unknown>;
    return {
      notifications: row.notifications === true,
      mic: row.mic === true,
    };
  } catch {
    return { notifications: false, mic: false };
  }
}

export function savePermissionMemory(memory: PermissionMemory): void {
  try {
    localStorage.setItem(PERMISSIONS_KEY, JSON.stringify(memory));
  } catch {
    // The in-memory flag still stops a second ask this session.
  }
}

/** Paint the reason, then open the OS dialog. The line is already visible. */
export async function askAfterLine(options: AskAfterLine): Promise<void> {
  await options.show(options.line);
  await options.paint();
  try {
    await options.request();
  } catch {
    // Dismissed or unavailable. No error tone.
  } finally {
    options.hide();
  }
}
