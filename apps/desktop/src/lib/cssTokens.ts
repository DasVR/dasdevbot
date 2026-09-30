/** Read motion tokens at animation start so JS easing stays on tokens.css. */

function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const sampleX = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sampleY = (t: number) => ((ay * t + by) * t + cy) * t;
  const sampleDX = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  const solveX = (x: number) => {
    let guess = x;
    for (let i = 0; i < 8; i += 1) {
      const error = sampleX(guess) - x;
      if (Math.abs(error) < 1e-6) {
        return guess;
      }
      const slope = sampleDX(guess);
      if (Math.abs(slope) < 1e-6) {
        break;
      }
      guess -= error / slope;
    }
    let lo = 0;
    let hi = 1;
    guess = x;
    for (let i = 0; i < 24; i += 1) {
      const xEst = sampleX(guess);
      if (Math.abs(xEst - x) < 1e-6) {
        return guess;
      }
      if (xEst < x) {
        lo = guess;
      } else {
        hi = guess;
      }
      guess = (lo + hi) / 2;
    }
    return guess;
  };
  return (x: number) => sampleY(solveX(x));
}

function linearEase(t: number): number {
  return t;
}

export function tokenMs(name: string, fallback: number): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const match = /^(-?\d*\.?\d+)(ms|s)?$/.exec(raw);
  if (!match) {
    return fallback;
  }
  const value = Number(match[1]);
  if (!Number.isFinite(value)) {
    return fallback;
  }
  return match[2] === "s" ? value * 1000 : value;
}

export function tokenEase(name: string): (t: number) => number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const match =
    /cubic-bezier\(\s*(-?\d*\.?\d+)\s*,\s*(-?\d*\.?\d+)\s*,\s*(-?\d*\.?\d+)\s*,\s*(-?\d*\.?\d+)\s*\)/.exec(
      raw,
    );
  if (!match) {
    return linearEase;
  }
  return cubicBezier(Number(match[1]), Number(match[2]), Number(match[3]), Number(match[4]));
}
