/** Hand-authored centerlines lifted from the look v1 approval-card mock. */
export const CHECK_PATH =
  "M4.2 12.9c1.5 1.3 3.1 3.1 4.6 5.3.3.4.8.4 1-.1 2.6-5.1 6-9.6 10.2-13.6";

export const STRIKE_PATH = "M2 7.4c19.6-1.8 40.2-2.6 61-2.2 21.4.4 43 1.8 65 3.6";

export const DENY_MARK_PATH = "M4.5 12.6c5-.7 10-.6 15 .2";

export const CHEVRON_PATH = "M4.5 2.5 8 6l-3.5 3.5";

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

function n(value: number): string {
  return value.toFixed(1);
}

/** Seeded wobble from beside the title to the risk strip. Length follows layout. */
export function arrowFromLayout(
  approvalId: string,
  card: DOMRect,
  strip: DOMRect,
  title: DOMRect,
): ArrowMark | null {
  const startX = title.right - card.left + 12;
  const startY = title.top - card.top + title.height * 0.42;
  const endX = strip.right - card.left - 36;
  const endY = strip.bottom - card.top - 1;
  const dx = endX - startX;
  const dy = endY - startY;
  const len = Math.hypot(dx, dy);
  if (len < 12 || card.width < 8 || card.height < 8) {
    return null;
  }
  const rng = unitRandom(hashSeed(approvalId));
  const px = -dy / len;
  const py = dx / len;
  const wobbleA = (rng() - 0.5) * 10;
  const wobbleB = (rng() - 0.5) * 8;
  const c1x = startX + dx * 0.34 + px * wobbleA;
  const c1y = startY + dy * 0.34 + py * wobbleA;
  const c2x = startX + dx * 0.7 + px * wobbleB;
  const c2y = startY + dy * 0.7 + py * wobbleB;
  const shaft = `M${n(startX)} ${n(startY)}C${n(c1x)} ${n(c1y)} ${n(c2x)} ${n(c2y)} ${n(endX)} ${n(endY)}`;
  const tx = dx / len;
  const ty = dy / len;
  const back = 8 + rng() * 2;
  const spread = 4.2 + rng() * 1.6;
  const bx = endX - tx * back;
  const by = endY - ty * back;
  const lx = bx + px * spread;
  const ly = by + py * spread;
  const rx = bx - px * spread;
  const ry = by - py * spread;
  const head = `M${n(lx)} ${n(ly)}C${n((lx + endX) / 2)} ${n((ly + endY) / 2)} ${n(endX - tx * 2)} ${n(endY - ty * 2)} ${n(endX)} ${n(endY)}C${n(endX - tx * 2)} ${n(endY - ty * 2)} ${n((rx + endX) / 2)} ${n((ry + endY) / 2)} ${n(rx)} ${n(ry)}`;
  return {
    width: Math.max(1, Math.round(card.width)),
    height: Math.max(1, Math.round(card.height)),
    shaft,
    head,
  };
}
