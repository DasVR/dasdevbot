/** Hand-authored centerlines lifted from the look v1 approval-card mock. */
export const CHECK_PATH =
  "M4.2 12.9c1.5 1.3 3.1 3.1 4.6 5.3.3.4.8.4 1-.1 2.6-5.1 6-9.6 10.2-13.6";

export const STRIKE_PATH = "M2 7.4c19.6-1.8 40.2-2.6 61-2.2 21.4.4 43 1.8 65 3.6";

export const DENY_MARK_PATH = "M4.5 12.6c5-.7 10-.6 15 .2";

export const CHEVRON_PATH = "M4.5 2.5 8 6l-3.5 3.5";

/**
 * Focus glyph. Concentric rings in a 16px viewBox, radii 5.6 and 2.1, drawn
 * at 1.5 with round caps and currentColor. Not the pen.
 */
export const FOCUS_OUTER_PATH = "M13.6 8a5.6 5.6 0 1 1-11.2 0 5.6 5.6 0 1 1 11.2 0";
export const FOCUS_INNER_PATH = "M10.1 8a2.1 2.1 0 1 1-4.2 0 2.1 2.1 0 1 1 4.2 0";

/** Enter. JetBrains Mono has no U+21B5, so the hint draws this 1.5 line icon. */
export const ENTER_KEY_PATH = "M12 3.25V9.25H4.25M7.15 6.35 4.25 9.25 7.15 12.15";

/** Delete. The latin subset has no U+232B, so the hint draws this 1.5 line icon. */
export const DELETE_KEY_PATH =
  "M13.75 4.25H6.6L3 8l3.6 3.75H13.75ZM8.15 6.15 11.35 9.85M11.35 6.15 8.15 9.85";

/** Quiet empty-state rule. One stroke, no card. */
export const QUIET_LINE_PATH = "M1.5 5.2c18-.7 36-.9 54-.3 16.4.5 32.2.9 48.5.2";

/** Reading trace. Authored once for a live tool step. */
export const READING_PATH =
  "M1.5 6.8c2.9-2.6 5.4-3 8-1 2.5 1.9 4.8 2.2 7.4.2 2.7-2 5.2-2.2 7.9-.3 2.4 1.7 4.7 1.9 7.3.1 2.3-1.5 4.6-1.9 7.2-.9";

/** Writing trace. Authored once for a live tool step. */
export const WRITING_PATH =
  "M1.6 8.6c2.3-.7 4.2-2.3 5.2-4.4.6-1.3-.4-2.2-1.3-1.2-1.4 1.6-1.1 4.6.7 5.6 1.9 1.1 4.1-.4 5.5-2.3 1-1.3 1.8-3.3.8-3.7-1-.4-1.8 1.9-1.3 3.6.6 2.2 3 2.8 5 1.5 1.9-1.2 3-3.2 3.9-4.8.6-1.1-.4-1.9-1.2-1-1.2 1.4-.9 4.1.8 5.1 2.2 1.2 4.9-.4 6.8-2";

/** Handoff mark. One pen stroke between two event ids. */
export const HAND_PATH =
  "M2 9.6c2.9-.1 5.2-1.4 6.9-3.6 1.1-1.5.2-3-1.2-2-1.7 1.2-1.5 4.3.6 5.3 2.5 1.2 5.6-.3 8.6-1.8 3.5-1.7 7.6-2.7 14.9-2.2";

export interface ArrowMark {
  width: number;
  height: number;
  shaft: string;
  head: string;
}

function hashSeed(id: string): number {
  let hash = 2166136261;
  for (let index = 0; index < id.length; index += 1) {
    hash ^= id.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function unitRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
    mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

const NUDGE_PX = 1.5;

function nudge(rng: () => number, x: number, y: number): string {
  const dx = (rng() - 0.5) * (NUDGE_PX * 2);
  const dy = (rng() - 0.5) * (NUDGE_PX * 2);
  return `${(x + dx).toFixed(2)} ${(y + dy).toFixed(2)}`;
}

/**
 * Mock shaft + head in a 58×46 box. Endpoints stay put so the tip lands on the
 * strip's lower edge; only the control points take a seeded ±1.5px nudge.
 */
export function arrowFromId(approvalId: string): ArrowMark {
  const rng = unitRandom(hashSeed(approvalId));
  const shaftC1 = nudge(rng, 46.1, 35.8);
  const shaftC2 = nudge(rng, 43.3, 29.7);
  const shaftC3 = nudge(rng, 34.3, 19.3);
  const shaftC4 = nudge(rng, 29.5, 15.1);
  const headC1 = nudge(rng, 28.7, 9.7);
  const headC2 = nudge(rng, 26.3, 9.0);
  const headC3 = nudge(rng, 24.5, 10.9);
  const headC4 = nudge(rng, 25.2, 13.4);
  return {
    width: 58,
    height: 46,
    shaft: `M47.2 42.6C${shaftC1} ${shaftC2} 38.4 24.0C${shaftC3} ${shaftC4} 24.2 8.2`,
    head: `M31.4 10.6C${headC1} ${headC2} 24.1 8.3C${headC3} ${headC4} 26.2 15.9`,
  };
}
