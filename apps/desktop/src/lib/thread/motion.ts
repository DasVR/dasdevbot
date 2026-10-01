import { linear } from "svelte/easing";
import type { AnimationConfig } from "svelte/animate";
import { flip } from "svelte/animate";
import type { TransitionConfig } from "svelte/transition";
import { tokenEase, tokenMs } from "../cssTokens";

export function prefersReducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Eases a bubble's height toward its text. The loop stops once it has settled,
 * so a finished reply does not animate at idle.
 */
export function easeHeight(node: HTMLElement): () => void {
  const inner = node.firstElementChild;
  if (!(inner instanceof HTMLElement) || prefersReducedMotion()) {
    return () => {};
  }
  let height = inner.offsetHeight;
  let frame = 0;
  let running = false;
  const settle = () => {
    const target = inner.offsetHeight;
    height += (target - height) * (1 - Math.exp(-16 / 75));
    if (Math.abs(target - height) < 0.4) {
      node.style.height = "";
      running = false;
      return;
    }
    node.style.height = `${height.toFixed(2)}px`;
    frame = requestAnimationFrame(settle);
  };
  const kick = () => {
    if (running) {
      return;
    }
    running = true;
    frame = requestAnimationFrame(settle);
  };
  const observer = new ResizeObserver(kick);
  observer.observe(inner);
  kick();
  return () => {
    cancelAnimationFrame(frame);
    observer.disconnect();
  };
}

/** Finished tool detail folds away. Reduced motion fades and does not resize. */
export function collapseHeight(node: Element): TransitionConfig {
  const height = node instanceof HTMLElement ? node.offsetHeight : 0;
  if (prefersReducedMotion()) {
    return {
      duration: tokenMs("--dur-fast", 140),
      easing: linear,
      css: (t) => `opacity: ${t};`,
    };
  }
  return {
    duration: tokenMs("--dur-base", 240),
    easing: tokenEase("--ease-out"),
    css: (t) => `height: ${(t * height).toFixed(2)}px; opacity: ${t}; overflow: hidden;`,
  };
}

/** New thread rows. Reduced motion is a linear fade and does not travel. */
export function arrive(_node: Element, params: { y?: number; play?: boolean } = {}): TransitionConfig {
  if (params.play === false) {
    return { duration: 0 };
  }
  const y = params.y ?? 8;
  if (prefersReducedMotion()) {
    return {
      duration: tokenMs("--dur-soft", 160),
      easing: linear,
      css: (t) => `opacity: ${t};`,
    };
  }
  return {
    duration: tokenMs("--dur-soft", 360),
    easing: tokenEase("--ease-out"),
    css: (t, u) => `opacity: ${t}; transform: translateY(${u * y}px);`,
  };
}

/** Existing rows step aside. Translate only. Reduced motion does not travel. */
export function stepRows(node: Element, coords: { from: DOMRect; to: DOMRect }): AnimationConfig {
  const dx = coords.from.left - coords.to.left;
  const dy = coords.from.top - coords.to.top;
  const moved = Math.abs(dx) >= 1 || Math.abs(dy) >= 1;
  if (prefersReducedMotion() || !moved) {
    return { duration: 0 };
  }
  const easeOut = tokenEase("--ease-out");
  const duration = tokenMs("--dur-soft", 360);
  const base = flip(node, coords, { duration, easing: easeOut });
  const css = base.css;
  if (!css) {
    return { duration: 0 };
  }
  return {
    ...base,
    duration,
    easing: easeOut,
    css: (t, u) => css(t, u).replace(/scale\([^)]*\);?$/, "scale(1, 1);"),
  };
}
