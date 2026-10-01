import type { ShellForm } from "./geometry";

/**
 * Tauri's invoke, present only inside the webview. The browser stage does not
 * load @tauri-apps/api; the review captures run there because Linux WebKit
 * will not show a transparent pill.
 */
interface TauriInternals {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
}

export interface NativeRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NativeTarget extends NativeRect {
  radius: number;
  glass: boolean;
  alwaysOnTop: boolean;
  resizable: boolean;
}

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function tauri(): TauriInternals | null {
  const host = window as unknown as { __TAURI_INTERNALS__?: TauriInternals };
  return host.__TAURI_INTERNALS__?.invoke ? host.__TAURI_INTERNALS__ : null;
}

function call<T>(cmd: string, args: Record<string, unknown>): Promise<T> | null {
  const internals = tauri();
  if (!internals) {
    return null;
  }
  return internals.invoke(cmd, args) as Promise<T>;
}

export function prepareForm(form: ShellForm): Promise<NativeTarget> | null {
  return call("prepare_shell_form", { form });
}

export function readMetrics(): Promise<NativeRect> | null {
  return call("shell_metrics", {});
}

export function setBounds(form: ShellForm, rect: NativeRect): Promise<void> | null {
  return call("set_shell_bounds", {
    form,
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
  });
}

export function windowCommand(
  name: "window_minimize" | "window_toggle_maximize" | "window_close",
): Promise<void> | null {
  return call(name, {});
}
