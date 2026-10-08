import type { ShellForm } from "./geometry";

export interface ShellStageApi {
  snap: (form: ShellForm) => void;
  morph: (form: ShellForm) => Promise<void>;
  start: () => void;
  form: () => ShellForm;
  /** A user-style form change: morph, then focus the new form's primary control. */
  go: (form: ShellForm) => Promise<void>;
}

export interface ShellCapApi {
  start: () => void;
  step: (dt: number) => Promise<void>;
  pending: () => number;
  running: () => boolean;
  /** The virtual capture clock in ms. Side-by-side stamps come from this. */
  now: () => number;
}

declare global {
  interface Window {
    __shellStage?: ShellStageApi;
    __shellCap?: ShellCapApi;
  }
}
