import type { ShellForm } from "./geometry";

export interface ShellStageApi {
  snap: (form: ShellForm) => void;
  morph: (form: ShellForm) => Promise<void>;
  start: () => void;
  form: () => ShellForm;
}

export interface ShellCapApi {
  start: () => void;
  step: (dt: number) => Promise<void>;
  pending: () => number;
  running: () => boolean;
}

declare global {
  interface Window {
    __shellStage?: ShellStageApi;
    __shellCap?: ShellCapApi;
  }
}
