/** Stage the shell mock was drawn on. The review clip is 1440×900. */
export const STAGE_WIDTH = 1440;
export const STAGE_HEIGHT = 900;

/** Roster column. The mock's `.roster` width. Not a space token. */
export const ROSTER_WIDTH = 272;

/** Custom titlebar. The mock's `.tbar` height. Not a space token. */
export const TITLEBAR_HEIGHT = 46;

/**
 * Titlebar icon button radius. Sits between --r-xs (6) and --r-sm (10).
 * The mock uses 9.
 */
export const TITLE_BUTTON_RADIUS = "9px";

/**
 * Mock traffic-light fill. It sits between --hairline and --hairline-strong
 * and is not itself a token. Windows builds hide these and use caption buttons.
 */
export const CHROME_DOT = "#D8CFC2";

/**
 * Desk wash behind the stage. OS scenery, not a product surface, so it is not
 * a paper token. The window itself stays on --paper-base.
 */
export const DESK_WASH = "linear-gradient(160deg, #EFE8DC 0%, #E9E1D3 46%, #E4DACB 100%)";

/**
 * Static paper grain from the shell mock. Painted once. Never animated.
 * There is no grain token.
 */
export const PAPER_GRAIN =
  "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='240' height='240'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='1.15' numOctaves='2' stitchTiles='stitch'/%3E%3CfeColorMatrix values='0 0 0 0 .227 0 0 0 0 .157 0 0 0 0 .086 .42 0 0 0 -.17'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E\")";

/**
 * Mock window shadow. Heavier than --shadow-float and adds a 12% edge ring.
 * The shade color still comes from the token.
 */
export const WINDOW_SHADOW =
  "0 0 0 1px rgb(var(--shade) / 0.12), 0 2px 6px rgb(var(--shade) / 0.06), 0 30px 70px -20px rgb(var(--shade) / 0.30)";

/** Mock `swap()` lifts the outgoing title by 3px. Dropped under reduced motion. */
export const TITLE_LIFT_PX = 3;

/** Mock `swap()` waits 40ms before the incoming title. Not a duration token. */
export const TITLE_SWAP_DELAY_MS = 40;

/** Mock digit roll leaves in 120ms. Not a duration token. */
export const ROLL_OUT_MS = 120;

/** Empty send control. The mock rests it at 0.45. No opacity token. */
export const SEND_REST_OPACITY = "0.45";

/** Mock cursor fade. Not a duration token. */
export const CURSOR_FADE_MS = 160;

/** Mock companion inset from the right (1440 − 1000 − 400). */
export const COMPANION_RIGHT = 40;

/** Mock companion inset from the top of the stage. */
export const COMPANION_TOP = 72;

/** Mock pill inset from the bottom (900 − 800 − 52). */
export const PILL_BOTTOM = 48;

/**
 * Companion composer inset. Mock: x is 12px inside the window (1012 − 1000)
 * and the composer sits 12px above the window's bottom edge.
 */
export const COMPOSER_INSET = 12;

/**
 * Mock `pressVisual` hold. It stays 140ms even when reduced motion
 * collapses --dur-fast to 0, so the click stays on the mock's clock.
 */
export const PRESS_HOLD_MS = 140;

/**
 * Mock toShell fractions of --dur-stage.
 * Companion content waits 35%. The pill content waits 40%.
 * The paper window leaves over 45% and returns over 60%.
 */
export const COMPANION_IN_AT = 0.35;
export const PILL_IN_AT = 0.4;
export const WINDOW_OUT_PORTION = 0.45;
export const WINDOW_IN_PORTION = 0.6;

export type ShellForm = "full" | "companion" | "pill";

export interface ShellRect {
  x: number;
  y: number;
  w: number;
  h: number;
  r: number;
}

/**
 * Mock `SH` rects, in stage pixels: window and the shared composer.
 * Radius 14 matches --r-md. 30 and 26 are not radius tokens; they are the
 * mock composer corners (full 30, companion and pill 26).
 */
export const MOCK_FORMS: Record<ShellForm, { win: ShellRect; composer: ShellRect }> = {
  full: {
    win: { x: 40, y: 52, w: 1360, h: 828, r: 14 },
    composer: { x: 516, y: 798, w: 680, h: 60, r: 30 },
  },
  companion: {
    win: { x: 1000, y: 72, w: 400, h: 790, r: 14 },
    composer: { x: 1012, y: 798, w: 376, h: 52, r: 26 },
  },
  pill: {
    win: { x: 520, y: 800, w: 400, h: 52, r: 26 },
    composer: { x: 520, y: 800, w: 400, h: 52, r: 26 },
  },
};

/** Background window on the mock desk. Scenery only. */
export const MOCK_OTHER = { x: 170, y: 120, w: 1100, h: 900 };

/** The review clip's stage. Only this box uses the mock's absolute SH coordinates. */
export function isMockStage(width: number, height: number): boolean {
  return width === STAGE_WIDTH && height === STAGE_HEIGHT;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * Unscaled mock sizes. Radii stay 14, 30, and 26.
 * Full fills the viewport. Companion is 400×790, 40px off the right, top 72.
 * The pill is 400×52, centered, 48px off the bottom. A viewport smaller than
 * the form fills that viewport instead of scaling.
 */
export function placeWin(form: ShellForm, box: { w: number; h: number }): ShellRect {
  const mock = MOCK_FORMS[form].win;
  if (form === "full") {
    return { x: 0, y: 0, w: box.w, h: box.h, r: mock.r };
  }
  const w = Math.min(mock.w, box.w);
  const h = Math.min(mock.h, box.h);
  if (form === "companion") {
    return {
      x: clamp(box.w - w - COMPANION_RIGHT, 0, Math.max(0, box.w - w)),
      y: clamp(COMPANION_TOP, 0, Math.max(0, box.h - h)),
      w,
      h,
      r: mock.r,
    };
  }
  if (form === "pill") {
    return {
      x: clamp((box.w - w) / 2, 0, Math.max(0, box.w - w)),
      y: clamp(box.h - h - PILL_BOTTOM, 0, Math.max(0, box.h - h)),
      w,
      h,
      r: mock.r,
    };
  }
  return assertForm(form);
}

/** Composer that belongs to a placed window. Radius is the mock's, never scaled. */
export function placeComposer(
  form: ShellForm,
  box: { w: number; h: number },
  win: ShellRect,
): ShellRect {
  const mock = MOCK_FORMS[form].composer;
  if (form === "full") {
    const width = Math.min(mock.w, Math.max(280, box.w - ROSTER_WIDTH - 48));
    const x = ROSTER_WIDTH + Math.max(24, (box.w - ROSTER_WIDTH - width) / 2);
    return { x, y: box.h - 22 - mock.h, w: width, h: mock.h, r: mock.r };
  }
  if (form === "pill") {
    return { x: win.x, y: win.y, w: win.w, h: win.h, r: mock.r };
  }
  if (form === "companion") {
    return {
      x: win.x + COMPOSER_INSET,
      y: win.y + win.h - mock.h - COMPOSER_INSET,
      w: Math.max(0, win.w - COMPOSER_INSET * 2),
      h: mock.h,
      r: mock.r,
    };
  }
  return assertForm(form);
}

export function shellSearch(): { stage: boolean; capture: boolean } {
  const params = new URLSearchParams(window.location.search);
  const capture = params.has("shellCapture");
  return { stage: capture || params.has("shellStage"), capture };
}

export function assertForm(form: never): never {
  throw new Error(`unhandled shell form ${String(form)}`);
}
