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

/**
 * Snap Layouts (CD ruling a): tell the Windows overlay where the drawn
 * maximize button is, in CSS px, or null when the captions are not shown.
 * The overlay answers HTMAXBUTTON there so the Win11 flyout appears.
 */
export function placeSnapOverlay(rect: NativeRect | null): Promise<void> | null {
  return call("snap_maximize_rect", { rect });
}

export function isMaximized(): Promise<boolean> | null {
  return call("plugin:window|is_maximized", { label: "main" });
}

/** What the pointer did on the Snap overlay, from src-tauri/src/snap.rs. */
export const SNAP_EVENT = "dasdevbot:snap-maximize";
export type SnapPointer = "hover" | "press" | "release" | "leave";

/**
 * Segoe Fluent Icons caption glyphs (Win11), with Segoe MDL2 Assets on
 * Windows 10. Only on Windows: elsewhere the drawn SVG stands in.
 */
export const CAPTION_GLYPHS = {
  minimize: "\uE921",
  maximize: "\uE922",
  restore: "\uE923",
  close: "\uE8BB",
} as const;

export function onWindows(): boolean {
  return typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);
}
