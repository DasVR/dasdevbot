/**
 * Pen mark #8, character B "Curl".
 * One pass: a pebble that ends in an open, off-center lift-off.
 * The flourish is not a stem and not a closed loop.
 * Seeded once so the stroke never re-jitters per frame.
 */

export const CURL_DRAW_MS = 820;
export const EYE_DRAW_MS = 120;
export const BLINK_MS = 6000;

export interface CurlMarks {
  body: string;
  eyes: readonly [string, string];
}

let draws = 0;

export function curlDraws(): number {
  return draws;
}

export function noteCurlDraw(): void {
  draws += 1;
}

function wobble(path: string, seed: number, amp: number): string {
  const next = unit(seed);
  return path.replace(/-?\d*\.?\d+/g, (token) => {
    const value = Number.parseFloat(token) + (next() - 0.5) * amp;
    return value.toFixed(2);
  });
}

function unit(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), 1 | state);
    mixed = (mixed + Math.imul(mixed ^ (mixed >>> 7), 61 | mixed)) ^ mixed;
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

/** Authored centerline from the welcome study, seed 83, then left alone. */
export function curlMarks(): CurlMarks {
  const body =
    "M24 12.2 C15.8 12.2 9.4 18.2 9.4 26 C9.4 33.8 15.8 40 24 40 C32.2 40 38.6 33.8 38.6 26 C38.6 18.2 32.2 12.2 25.2 12.2 C23.2 10 23.6 7 25.8 6.6 C27.6 6.3 28.6 8 27.4 9.2";
  return {
    body: wobble(body, 83, 0.45),
    eyes: [wobble("M19.8 23.4 L19.8 28", 94, 0.25), wobble("M28.2 23.4 L28.2 28", 95, 0.25)],
  };
}
