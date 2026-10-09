import { tokenEase, tokenMs } from "../cssTokens";
import {
  COMPANION_IN_AT,
  CURSOR_FADE_MS,
  PRESS_HOLD_MS,
  MOCK_FORMS,
  PILL_IN_AT,
  ROLL_OUT_MS,
  ROSTER_WIDTH,
  type ShellForm,
  type ShellRect,
  TITLE_LIFT_PX,
  TITLE_SWAP_DELAY_MS,
  WINDOW_IN_PORTION,
  WINDOW_OUT_PORTION,
  placeComposer,
} from "./geometry";
import { inTauri, prepareForm, readMetrics, setBounds, type NativeRect, type NativeTarget } from "./native";

const STOPPED = Symbol("stopped");
/** UID 5: reduced-motion cross-fade, linear, inside the 120-140ms band. */
const REDUCED_FADE_MS = 130;

interface Tracked {
  anim: Animation;
  origin: number;
  end: number;
  resolve: () => void;
}

class MotionClock {
  readonly capture: boolean;
  t = 0;
  anims: Tracked[] = [];
  private timers = new Map<number, { at: number; fn: () => void }>();
  private live = new Map<number, number>();
  private rafs: Array<() => void> = [];
  private adopted = new WeakMap<Animation, number>();
  private seq = 0;

  constructor(capture: boolean) {
    this.capture = capture;
  }

  now(): number {
    return this.capture ? this.t : performance.now();
  }

  after(ms: number, fn: () => void): number {
    const id = ++this.seq;
    if (!this.capture) {
      const handle = window.setTimeout(() => {
        this.live.delete(id);
        fn();
      }, ms);
      this.live.set(id, handle);
      return id;
    }
    this.timers.set(id, { at: this.t + Math.max(0, ms), fn });
    return id;
  }

  raf(fn: () => void): void {
    if (!this.capture) {
      window.requestAnimationFrame(() => fn());
      return;
    }
    this.rafs.push(fn);
  }

  play(
    el: HTMLElement,
    frames: Keyframe[],
    ms: number,
    ease: string,
    delay = 0,
    fill: FillMode = "both",
  ): Promise<void> {
    const anim = el.animate(frames, {
      duration: Math.max(0, ms),
      delay,
      easing: ease,
      fill,
    });
    if (!this.capture) {
      return anim.finished.then(
        () => {
          try {
            anim.commitStyles();
          } catch {
            // The node can detach before the animation ends.
          }
          anim.cancel();
        },
        () => {},
      );
    }
    anim.pause();
    anim.currentTime = 0;
    return new Promise((resolve) => {
      this.anims.push({
        anim,
        origin: this.t,
        end: delay + Math.max(0, ms),
        resolve,
      });
    });
  }

  /**
   * Cancel only the animations running on these elements. Unlike cancelAll it
   * leaves timers alone, so a sequence sleep (the reduced cursor jump) lives.
   */
  cancelOn(els: Element[]): void {
    const set = new Set(els);
    const stay: Tracked[] = [];
    for (const tracked of this.anims) {
      const target = (tracked.anim.effect as KeyframeEffect | null)?.target ?? null;
      if (target && set.has(target)) {
        tracked.anim.cancel();
        tracked.resolve();
      } else {
        stay.push(tracked);
      }
    }
    this.anims = stay;
    if (!this.capture) {
      for (const el of els) {
        for (const anim of el.getAnimations()) {
          anim.cancel();
        }
      }
    }
  }

  cancelAll(): void {
    for (const tracked of this.anims) {
      tracked.anim.cancel();
      tracked.resolve();
    }
    this.anims = [];
    if (!this.capture) {
      for (const handle of this.live.values()) {
        window.clearTimeout(handle);
      }
      this.live.clear();
    }
    this.timers.clear();
    this.rafs = [];
    this.adopted = new WeakMap();
  }

  get pending(): number {
    return this.timers.size + this.live.size + this.rafs.length + this.anims.length;
  }

  private sync(): void {
    const stay: Tracked[] = [];
    for (const tracked of this.anims) {
      const local = this.t - tracked.origin;
      if (local >= tracked.end) {
        tracked.anim.currentTime = tracked.end;
        try {
          tracked.anim.commitStyles();
        } catch {
          // Detached during the step.
        }
        tracked.anim.cancel();
        tracked.resolve();
      } else {
        tracked.anim.currentTime = Math.max(0, local);
        stay.push(tracked);
      }
    }
    this.anims = stay;
  }

  private adoptRest(): void {
    const owned = new Set(this.anims.map((tracked) => tracked.anim));
    for (const anim of document.getAnimations()) {
      if (owned.has(anim) || this.adopted.has(anim)) {
        continue;
      }
      anim.pause();
      this.adopted.set(anim, this.t);
    }
    for (const anim of document.getAnimations()) {
      const origin = this.adopted.get(anim);
      if (origin == null || owned.has(anim)) {
        continue;
      }
      const local = this.t - origin;
      const end = anim.effect?.getComputedTiming().endTime ?? 0;
      if (typeof end === "number" && Number.isFinite(end) && local >= end) {
        anim.finish();
        this.adopted.delete(anim);
      } else {
        anim.currentTime = Math.max(0, local);
      }
    }
  }

  async step(dt: number): Promise<void> {
    if (!this.capture) {
      return;
    }
    const target = this.t + dt;
    for (;;) {
      let pick: { id: number; at: number; fn: () => void } | null = null;
      for (const [id, timer] of this.timers) {
        if (timer.at <= target && (pick == null || timer.at < pick.at)) {
          pick = { id, at: timer.at, fn: timer.fn };
        }
      }
      if (!pick) {
        break;
      }
      this.timers.delete(pick.id);
      this.t = Math.max(this.t, pick.at);
      this.sync();
      pick.fn();
      await flush();
      this.adoptRest();
    }
    this.t = target;
    const queued = this.rafs.splice(0);
    for (const fn of queued) {
      fn();
    }
    await flush();
    this.sync();
    this.adoptRest();
    await flush();
  }
}

async function flush(): Promise<void> {
  for (let i = 0; i < 24; i += 1) {
    await Promise.resolve();
  }
}

function easeCss(name: string): string {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return raw || "linear";
}

function applyRect(el: HTMLElement, rect: ShellRect): void {
  el.style.left = `${rect.x}px`;
  el.style.top = `${rect.y}px`;
  el.style.width = `${rect.w}px`;
  el.style.height = `${rect.h}px`;
  el.style.borderRadius = `${rect.r}px`;
}

/**
 * UID 1 (native race): a composer pinned to the window's edges by insets, so
 * it can be laid into whatever size the native window has on this frame.
 */
interface Insets {
  l: number;
  r: number;
  b: number;
  h: number;
  rad: number;
}

function insetsOf(rect: ShellRect, box: { w: number; h: number }): Insets {
  return { l: rect.x, r: box.w - rect.x - rect.w, b: box.h - rect.y - rect.h, h: rect.h, rad: rect.r };
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function lerpInsets(a: Insets, b: Insets, t: number): Insets {
  return { l: lerp(a.l, b.l, t), r: lerp(a.r, b.r, t), b: lerp(a.b, b.b, t), h: lerp(a.h, b.h, t), rad: lerp(a.rad, b.rad, t) };
}

/** Lay insets into a box. Never leaves the box, so nothing is ever clipped. */
export function placeIn(ins: Insets, box: { w: number; h: number }): ShellRect {
  const x = Math.min(Math.max(0, ins.l), box.w);
  const w = Math.max(0, box.w - x - Math.max(0, ins.r));
  const h = Math.max(0, Math.min(ins.h, box.h));
  const y = Math.max(0, box.h - Math.max(0, ins.b) - h);
  return { x, y, w, h, r: ins.rad };
}

function rectFrames(rect: ShellRect): Keyframe {
  return {
    left: `${rect.x}px`,
    top: `${rect.y}px`,
    width: `${rect.w}px`,
    height: `${rect.h}px`,
    borderRadius: `${rect.r}px`,
  };
}

export interface ShellNodes {
  win: HTMLElement;
  composer: HTMLElement;
  roster: HTMLElement;
  stream: HTMLElement;
  titleFull: HTMLElement;
  titleComp: HTMLElement;
  layerFull: HTMLElement;
  layerComp: HTMLElement;
  layerPill: HTMLElement;
  count: HTMLElement;
  cursor: HTMLElement | null;
  toComp: HTMLElement;
  toPill: HTMLElement;
}

export interface ShellMotion {
  form: () => ShellForm;
  snap: (form: ShellForm) => void;
  morph: (form: ShellForm) => Promise<void>;
  layout: () => void;
  startSequence: () => void;
  /** The capture clock in ms (virtual under ?shellCapture). */
  now: () => number;
  step: (dt: number) => Promise<void>;
  get running(): boolean;
  get pending(): number;
  destroy: () => void;
}

export function createShellMotion(
  nodes: ShellNodes,
  options: {
    stage: boolean;
    capture: boolean;
    reduced: () => boolean;
    onForm: (form: ShellForm) => void;
    /** UID 2: Builder's roster row flips to "waiting on you" on the count bump. */
    onBuilderAsks?: (asks: boolean) => void;
  },
): ShellMotion {
  const clock = new MotionClock(options.capture);
  let form: ShellForm = "full";
  let generation = 0;
  let running = false;
  let animating = false;
  let cursor = { x: 1120, y: 470 };
  const native = inTauri() && !options.stage;

  function viewport(): { w: number; h: number } {
    return { w: window.innerWidth, h: window.innerHeight };
  }

  function scale(): number {
    // Rects are never scaled, including on the 1440×900 clip (scale is 1 there).
    // The cursor uses the same pixels as the windows.
    return 1;
  }

  /**
   * The browser stage paints the mock's SH rects in CSS pixels at every
   * viewport, never scaled (G12). At 1280×800 the full window still sits at
   * 40,52 and runs past the edge, exactly as the mock does.
   */
  function winRect(next: ShellForm, box = viewport()): ShellRect {
    if (!options.stage) {
      return { x: 0, y: 0, w: box.w, h: box.h, r: MOCK_FORMS[next].win.r };
    }
    return { ...MOCK_FORMS[next].win };
  }

  /**
   * `box` is the window size the composer is laid out for. On the native
   * window that is the form's target size, which can differ from the
   * viewport until the OS resize lands; the composer is then pinned to the
   * window edges by the target's insets, so it is never clipped (UID 1).
   */
  function composerRect(next: ShellForm, box = viewport()): ShellRect {
    if (!options.stage) {
      const rect = placeComposer(next, box, winRect(next, box));
      const now = viewport();
      if (now.w === box.w && now.h === box.h) {
        return rect;
      }
      return placeIn(insetsOf(rect, box), now);
    }
    return { ...MOCK_FORMS[next].composer };
  }

  function opacity(el: HTMLElement, value: number): void {
    el.style.opacity = String(value);
  }

  function concealWindow(hidden: boolean): void {
    nodes.win.inert = hidden;
    nodes.win.style.visibility = hidden ? "hidden" : "";
  }

  function syncLay(): void {
    nodes.win.style.setProperty("--lay-full", `${winRect("full").w}px`);
  }

  function streamLayout(next: ShellForm): void {
    if (next === "full") {
      // Same math as the mock's right-anchored `.lay.full`: the column stays
      // pinned to the window's right edge and slides off the left as it narrows.
      nodes.stream.style.left = `calc(100% - var(--lay-full) + ${ROSTER_WIDTH}px)`;
      nodes.stream.style.width = `calc(var(--lay-full) - ${ROSTER_WIDTH}px)`;
      nodes.stream.style.right = "auto";
      nodes.stream.style.padding = "0";
      return;
    }
    nodes.stream.style.left = "0px";
    nodes.stream.style.width = "auto";
    nodes.stream.style.right = "0px";
    nodes.stream.style.padding = next === "companion" ? "18px 18px 76px" : "0";
  }

  function reserveCardClearance(next: ShellForm, target?: { w: number; h: number }): void {
    const box = target ?? viewport();
    const rect = options.stage ? composerRect(next) : placeComposer(next, box, winRect(next, box));
    const block = Math.max(0, box.h - rect.y);
    document.documentElement.style.setProperty("--composer-block", `${block}px`);
  }

  /** The native size the current form is heading for, while it has not landed. */
  let nativeBox: { w: number; h: number } | null = null;

  /**
   * Lay the composer out for `box` until the OS resize lands (or a second
   * passes, if the OS clamps the size), so a lagging resize never clips it.
   */
  function holdNative(box: { w: number; h: number } | null | undefined): void {
    if (!box) {
      return;
    }
    nativeBox = box;
    window.setTimeout(() => {
      if (nativeBox === box) {
        nativeBox = null;
        if (!tween && !animating) {
          paint(form);
        }
      }
    }, 1000);
  }

  function paint(next: ShellForm): void {
    nodes.win.dataset.shellForm = next;
    syncLay();
    const now = viewport();
    if (nativeBox && Math.abs(nativeBox.w - now.w) < 1 && Math.abs(nativeBox.h - now.h) < 1) {
      nativeBox = null;
    }
    const box = nativeBox ?? now;
    applyRect(nodes.win, winRect(next));
    applyRect(nodes.composer, composerRect(next, box));
    reserveCardClearance(next, box);
    const pill = next === "pill";
    const full = next === "full";
    opacity(nodes.win, pill ? 0 : 1);
    nodes.win.style.pointerEvents = pill ? "none" : "auto";
    concealWindow(pill);
    opacity(nodes.roster, full ? 1 : 0);
    opacity(nodes.titleFull, next === "companion" ? 0 : 1);
    opacity(nodes.titleComp, next === "companion" ? 1 : 0);
    opacity(nodes.layerFull, full ? 1 : 0);
    opacity(nodes.layerComp, next === "companion" ? 1 : 0);
    opacity(nodes.layerPill, pill ? 1 : 0);
    hits(next);
    concealStream(pill);
    opacity(nodes.stream, pill ? 0 : 1);
    streamLayout(next);
    form = next;
    options.onForm(next);
  }

  /**
   * Only the current form's controls can be reached. Hidden composer layers
   * and the collapsed roster are inert, so a hidden "1 waiting" is never
   * tabbable.
   */
  function hits(next: ShellForm): void {
    nodes.layerFull.style.pointerEvents = next === "full" ? "auto" : "none";
    nodes.layerComp.style.pointerEvents = next === "companion" ? "auto" : "none";
    nodes.layerPill.style.pointerEvents = next === "pill" ? "auto" : "none";
    nodes.layerFull.inert = next !== "full";
    nodes.layerComp.inert = next !== "companion";
    nodes.layerPill.inert = next !== "pill";
    nodes.roster.inert = next !== "full";
  }

  /**
   * The thread is inert while it is folded into the pill. The companion keeps
   * its own reflowed thread live: its only control is the waiting step, and
   * the main window holds no decision control in any form.
   */
  function concealStream(collapsed: boolean): void {
    nodes.stream.inert = collapsed;
  }

  /** Layers whose opacity a morph changes. */
  function fadeLayers(): HTMLElement[] {
    return [
      nodes.win,
      nodes.roster,
      nodes.titleFull,
      nodes.titleComp,
      nodes.layerFull,
      nodes.layerComp,
      nodes.layerPill,
      nodes.stream,
    ];
  }

  function readOpacity(el: HTMLElement): number {
    const value = Number.parseFloat(getComputedStyle(el).opacity);
    return Number.isFinite(value) ? value : 1;
  }

  /**
   * Reduced motion: after paint() has snapped the geometry and set the target
   * opacities, fade every changed layer linearly from where it was. A window
   * folding into the pill stays visible until its fade ends.
   */
  function crossFade(layers: HTMLElement[], before: number[], next: ShellForm): Promise<void> {
    const runs: Promise<void>[] = [];
    const winAt = layers.indexOf(nodes.win);
    const winFrom = winAt >= 0 ? before[winAt] : 1;
    const winTo = readOpacity(nodes.win);
    const winFades = winAt >= 0 && Math.abs(winTo - winFrom) >= 0.001;
    layers.forEach((el, i) => {
      const to = readOpacity(el);
      if (Math.abs(to - before[i]) < 0.001) {
        return;
      }
      if (winFades && el !== nodes.win && nodes.win.contains(el)) {
        // Inside a fading window a layer reads at the window's opacity, as in
        // the mock, never window x layer. Opening, it is already at its end;
        // folding, it holds where it was until the window has faded.
        if (winTo < winFrom) {
          runs.push(clock.play(el, [{ opacity: before[i] }, { opacity: before[i] }], REDUCED_FADE_MS, "linear", 0, "none"));
        }
        return;
      }
      if (el === nodes.win && next === "pill") {
        nodes.win.style.visibility = "";
      }
      runs.push(clock.play(el, [{ opacity: before[i] }, { opacity: to }], REDUCED_FADE_MS, "linear", 0, "both"));
    });
    return Promise.all(runs).then(() => {
      if (form === "pill" && next === "pill") {
        concealWindow(true);
      }
    });
  }

  function fade(el: HTMLElement, to: number, ms: number, delay = 0): Promise<void> {
    const from = Number.parseFloat(getComputedStyle(el).opacity);
    const start = Number.isFinite(from) ? from : 1;
    const ease = to > start ? easeCss("--ease-out") : easeCss("--ease-exit");
    return clock.play(
      el,
      [{ opacity: start }, { opacity: to }],
      ms,
      ease,
      delay,
      delay > 0 ? "forwards" : "both",
    );
  }

  function swapTitle(outEl: HTMLElement, inEl: HTMLElement, fast: number, base: number): void {
    if (options.reduced()) {
      void clock.play(outEl, [{ opacity: 1 }, { opacity: 0 }], REDUCED_FADE_MS, "linear", 0, "both");
      void clock.play(inEl, [{ opacity: 0 }, { opacity: 1 }], REDUCED_FADE_MS, "linear", 0, "both");
      return;
    }
    const lift = TITLE_LIFT_PX;
    void clock.play(
      outEl,
      [
        { opacity: 1, transform: "none" },
        { opacity: 0, transform: `translateY(${-lift}px)` },
      ],
      fast,
      easeCss("--ease-exit"),
    );
    void clock.play(
      inEl,
      [
        { opacity: 0, transform: `translateY(${lift}px)` },
        { opacity: 1, transform: "none" },
      ],
      base,
      easeCss("--ease-out"),
      TITLE_SWAP_DELAY_MS,
      "forwards",
    );
  }

  interface NativePlan {
    target: NativeTarget;
    from: NativeRect;
  }

  /** UID 1: the form's native target, read before any tween target is chosen. */
  async function planNative(next: ShellForm): Promise<NativePlan | null> {
    if (!native) {
      return null;
    }
    const prepared = prepareForm(next);
    const current = readMetrics();
    if (!prepared || !current) {
      return null;
    }
    const [target, from] = await Promise.all([prepared, current]);
    return { target, from };
  }

  /**
   * The in-flight native morph: composer insets and window radius at both
   * ends, and the eased progress. Every frame (and every resize the OS
   * delivers) lays them into the window's current size.
   */
  let tween: {
    from: Insets;
    to: Insets;
    fromBox: { w: number; h: number };
    toBox: { w: number; h: number };
    fromR: number;
    toR: number;
    e: number;
  } | null = null;

  /**
   * F1: how far the window the composer is laid into has actually got. The OS
   * resize lags the tween, so insets eased by time and laid into a window
   * that has not grown yet squeeze the composer (400 -> 20px, y=0). Easing
   * them by the window's own progress keeps every frame a real composer.
   */
  function boxProgress(box: { w: number; h: number }): number {
    if (!tween) {
      return 1;
    }
    const dw = tween.toBox.w - tween.fromBox.w;
    const dh = tween.toBox.h - tween.fromBox.h;
    const [start, delta, now] = Math.abs(dw) >= Math.abs(dh) ? [tween.fromBox.w, dw, box.w] : [tween.fromBox.h, dh, box.h];
    if (Math.abs(delta) < 1) {
      return tween.e;
    }
    return Math.min(1, Math.max(0, (now - start) / delta));
  }

  function placeTween(): void {
    if (!tween) {
      return;
    }
    const box = viewport();
    applyRect(nodes.composer, placeIn(lerpInsets(tween.from, tween.to, boxProgress(box)), box));
    applyRect(nodes.win, { x: 0, y: 0, w: box.w, h: box.h, r: lerp(tween.fromR, tween.toR, tween.e) });
  }

  async function tweenNative(next: ShellForm, ms: number, plan?: NativePlan | null): Promise<void> {
    const ready = plan === undefined ? await planNative(next) : plan;
    if (!ready) {
      return;
    }
    const { target, from } = ready;
    if (options.reduced() || ms <= 0) {
      await setBounds(next, target);
      return;
    }
    const ease = tokenEase("--ease-out");
    // F1: the clock starts when the tween paints its first frame. Coming out
    // of the pill that frame can land ~380ms after the morph starts, and a
    // clock started earlier opened the tween at p=0.72: one 746px jump.
    let t0: number | null = null;
    await new Promise<void>((resolve) => {
      const frame = () => {
        t0 ??= clock.now();
        const p = Math.min(1, (clock.now() - t0) / ms);
        const e = ease(p);
        const rect: NativeRect = {
          x: from.x + (target.x - from.x) * e,
          y: from.y + (target.y - from.y) * e,
          width: from.width + (target.width - from.width) * e,
          height: from.height + (target.height - from.height) * e,
        };
        void setBounds(next, rect);
        if (tween) {
          tween.e = e;
          placeTween();
        }
        if (p < 1) {
          clock.raf(frame);
        } else {
          resolve();
        }
      };
      clock.raf(frame);
    });
  }

  async function morph(next: ShellForm): Promise<void> {
    const from = form;
    if (from === next || animating) {
      return;
    }
    animating = true;
    try {
    // UID 1: on the native window, wait for the form's target size before
    // choosing any tween target, so nothing is laid out for a stale viewport.
    const plan = await planNative(next);
    const targetBox = plan ? { w: plan.target.width, h: plan.target.height } : undefined;
    nodes.win.dataset.shellForm = next;
    syncLay();
    reserveCardClearance(next, targetBox);
    if (options.reduced()) {
      // F2: cancel only this morph's own layers. clock.cancelAll() also
      // cleared the sequence's timers, so the un-awaited reduced cursor move
      // (a sleep, then a jump) never landed at (1060,640) or (1180,470).
      const layers = fadeLayers();
      clock.cancelOn([...layers, nodes.composer]);
      // UID 5: geometry snaps; the layers cross-fade linearly instead of popping.
      const before = layers.map((el) => readOpacity(el));
      holdNative(targetBox);
      paint(next);
      const fades = crossFade(layers, before, next);
      // F2: the morph ends when the mock's does: its reduced toShell awaits
      // the rect morph over --dur-stage (160ms reduced), not the 130ms fade,
      // so ending on the fade put every later beat 2 frames early (-6 by the
      // cursor fade).
      // An empty animation, not a timer: like the mock's rect morph it ends on
      // a frame boundary, so the next beat starts on the mock's frame.
      const ends = clock.play(nodes.win, [{}, {}], tokenMs("--dur-stage", 160), "linear", 0, "none");
      await tweenNative(next, 0, plan);
      if (native && form === next) {
        paint(next);
      }
      await Promise.all([fades, ends]);
      return;
    }
    hits(next);
    if (next === "pill") {
      concealStream(true);
    }
    const fast = tokenMs("--dur-fast", 140);
    const soft = tokenMs("--dur-soft", 360);
    const base = tokenMs("--dur-base", 240);
    const stageMs = tokenMs("--dur-stage", 520);
    const outEase = easeCss("--ease-out");
    const fromWin = winRect(from);
    const toWin = winRect(next, targetBox);
    const fromCmp = composerRect(from);
    // F1: the composer is laid into the target window, never squeezed into the
    // window it is leaving. composerRect(next, targetBox) pins the target
    // rect back into the current 400x52 viewport, and the insets of that
    // against targetBox leave a degenerate end (20px wide, y=0).
    const toCmp = targetBox ? placeComposer(next, targetBox, winRect(next, targetBox)) : composerRect(next);

    if (from === "full" && next === "companion") {
      void fade(nodes.roster, 0, fast);
      void fade(nodes.stream, 0, fast);
      clock.after(fast, () => streamLayout("companion"));
      void fade(nodes.stream, 1, soft, stageMs * COMPANION_IN_AT);
      swapTitle(nodes.titleFull, nodes.titleComp, fast, base);
      void fade(nodes.layerFull, 0, fast);
      void fade(nodes.layerComp, 1, soft, stageMs * COMPANION_IN_AT);
    } else if (from === "companion" && next === "full") {
      void fade(nodes.stream, 0, fast);
      clock.after(fast, () => streamLayout("full"));
      void fade(nodes.stream, 1, soft, stageMs * COMPANION_IN_AT);
      void fade(nodes.roster, 1, soft, stageMs * COMPANION_IN_AT);
      swapTitle(nodes.titleComp, nodes.titleFull, fast, base);
      void fade(nodes.layerComp, 0, fast);
      void fade(nodes.layerFull, 1, soft, stageMs * COMPANION_IN_AT);
    } else if (next === "pill") {
      nodes.win.inert = true;
      void fade(nodes.win, 0, stageMs * WINDOW_OUT_PORTION).then(() => {
        if (form === "pill") {
          nodes.win.style.visibility = "hidden";
        }
      });
      nodes.win.style.pointerEvents = "none";
      void fade(from === "companion" ? nodes.layerComp : nodes.layerFull, 0, fast);
      void fade(nodes.layerPill, 1, soft, stageMs * PILL_IN_AT);
    } else if (from === "pill") {
      nodes.win.style.visibility = "";
      nodes.win.inert = true;
      opacity(nodes.win, 0);
      opacity(nodes.titleFull, next === "full" ? 1 : 0);
      opacity(nodes.titleComp, next === "companion" ? 1 : 0);
      opacity(nodes.layerFull, 0);
      opacity(nodes.layerComp, 0);
      opacity(nodes.roster, next === "full" ? 1 : 0);
      streamLayout(next);
      opacity(nodes.stream, 1);
      void fade(nodes.win, 1, stageMs * WINDOW_IN_PORTION).then(() => {
        if (form !== "pill") {
          nodes.win.inert = false;
        }
      });
      nodes.win.style.pointerEvents = "auto";
      void fade(nodes.layerPill, 0, fast);
      void fade(next === "companion" ? nodes.layerComp : nodes.layerFull, 1, soft, stageMs * COMPANION_IN_AT);
    }

    form = next;
    options.onForm(next);
    if (plan && targetBox) {
      // UID 1: the composer and window follow the native window's real rect
      // every frame instead of a WAAPI tween whose targets were fixed up front.
      const startBox = viewport();
      tween = {
        from: insetsOf(fromCmp, startBox),
        to: insetsOf(toCmp, targetBox),
        fromBox: startBox,
        toBox: targetBox,
        fromR: fromWin.r,
        toR: toWin.r,
        e: 0,
      };
      placeTween();
      holdNative(targetBox);
      await tweenNative(next, stageMs, plan);
      tween = null;
      if (form === next) {
        paint(next);
      }
    } else {
      await Promise.all([
        clock.play(nodes.win, [rectFrames(fromWin), rectFrames(toWin)], stageMs, outEase),
        clock.play(nodes.composer, [rectFrames(fromCmp), rectFrames(toCmp)], stageMs, outEase),
      ]);
    }
    } finally {
      animating = false;
      tween = null;
      concealStream(form === "pill");
    }
  }

  function point(x: number, y: number): { x: number; y: number } {
    const factor = scale();
    return { x: x * factor, y: y * factor };
  }

  function placeCursor(x: number, y: number): void {
    cursor = { x, y };
    if (!nodes.cursor) {
      return;
    }
    const at = point(x, y);
    if (options.reduced()) {
      nodes.cursor.style.left = `${at.x - 3}px`;
      nodes.cursor.style.top = `${at.y - 2}px`;
      nodes.cursor.style.transform = "none";
      return;
    }
    nodes.cursor.style.transform = `translate(${at.x - 3}px, ${at.y - 2}px)`;
  }

  /** The mock's cursor arc (4a moveTo `arc = .08`): a hand, not a ruler. */
  const CURSOR_ARC = 0.08;

  function moveCursor(x: number, y: number, ms: number, gen: number): Promise<void> {
    if (options.reduced()) {
      return sleep(ms, gen).then(() => {
        placeCursor(x, y);
      });
    }
    const x0 = cursor.x;
    const y0 = cursor.y;
    const dx = x - x0;
    const dy = y - y0;
    const ease = tokenEase("--ease-in-out");
    return new Promise((resolve, reject) => {
      const t0 = clock.now();
      const frame = () => {
        if (generation !== gen) {
          reject(STOPPED);
          return;
        }
        const p = ease(ms <= 0 ? 1 : Math.min(1, (clock.now() - t0) / ms));
        // The mock's moveTo: eased progress along a shallow arc, bowed
        // perpendicular to the path by sin(pi * p) * CURSOR_ARC.
        const bow = Math.sin(Math.PI * p) * CURSOR_ARC;
        placeCursor(x0 + dx * p - dy * bow, y0 + dy * p + dx * bow);
        if (p >= 1) {
          placeCursor(x, y);
        }
        if (p < 1) {
          clock.raf(frame);
        } else {
          resolve();
        }
      };
      clock.raf(frame);
    });
  }

  function center(el: HTMLElement): { x: number; y: number } {
    const rect = el.getBoundingClientRect();
    const factor = scale();
    return {
      x: (rect.left + rect.width / 2) / factor,
      y: (rect.top + rect.height / 2) / factor,
    };
  }

  /** The cursor's glyph. It is an <svg>, so not an HTMLElement. */
  function cursorGlyph(): SVGElement | HTMLElement | null {
    const glyph = nodes.cursor?.firstElementChild;
    return glyph instanceof SVGElement || glyph instanceof HTMLElement ? glyph : null;
  }

  async function press(el: HTMLElement, ms: number, gen: number): Promise<void> {
    el.classList.add("is-press");
    // The mock's pressVisual squeezes the pointer to .92 about its tip. The
    // old HTMLElement check never matched the <svg>, so the squeeze never ran.
    const glyph = cursorGlyph();
    if (glyph && !options.reduced()) {
      glyph.style.scale = "0.92";
    }
    await sleep(ms, gen);
    el.classList.remove("is-press");
    if (glyph) {
      glyph.style.scale = "";
    }
  }

  function sleep(ms: number, gen: number): Promise<void> {
    return new Promise((resolve, reject) => {
      clock.after(ms, () => {
        if (generation !== gen) {
          reject(STOPPED);
        } else {
          resolve();
        }
      });
    });
  }

  function showCursor(on: boolean, gen: number): Promise<void> {
    if (!nodes.cursor) {
      return sleep(CURSOR_FADE_MS, gen);
    }
    const ms = options.reduced() ? 120 : CURSOR_FADE_MS;
    return clock.play(
      nodes.cursor,
      [{ opacity: on ? 0 : 1 }, { opacity: on ? 1 : 0 }],
      ms,
      on ? easeCss("--ease-out") : easeCss("--ease-exit"),
    );
  }

  function roll(value: string): void {
    if (options.reduced()) {
      // UID 5: the digit swaps at the same instant as the full-motion roll
      // (ROLL_OUT_MS), as a linear cross-fade centred on that instant.
      const half = REDUCED_FADE_MS / 2;
      void clock
        .play(nodes.count, [{ opacity: 1 }, { opacity: 0 }], half, "linear", ROLL_OUT_MS - half, "none")
        .then(() => {
          nodes.count.textContent = value;
          return clock.play(nodes.count, [{ opacity: 0 }, { opacity: 1 }], half, "linear", 0, "none");
        });
      return;
    }
    void clock
      .play(
        nodes.count,
        [
          { opacity: 1, transform: "none" },
          { opacity: 0, transform: "translateY(-5px)" },
        ],
        ROLL_OUT_MS,
        easeCss("--ease-exit"),
        0,
        "none",
      )
      .then(() => {
        nodes.count.textContent = value;
        return clock.play(
          nodes.count,
          [
            { opacity: 0, transform: "translateY(5px)" },
            { opacity: 1, transform: "none" },
          ],
          tokenMs("--dur-base", 240),
          easeCss("--ease-out"),
          0,
          "none",
        );
      });
  }

  async function sequence(gen: number): Promise<void> {
    running = true;
    try {
      placeCursor(1120, 470);
      await sleep(300, gen);
      void showCursor(true, gen).catch((error: unknown) => {
        if (error !== STOPPED) {
          throw error;
        }
      });
      await sleep(420, gen);
      const compAt = center(nodes.toComp);
      await moveCursor(compAt.x, compAt.y, 560, gen);
      await press(nodes.toComp, PRESS_HOLD_MS, gen);
      await morph("companion");
      await sleep(900, gen);
      const pillAt = center(nodes.toPill);
      await moveCursor(pillAt.x, pillAt.y, 480, gen);
      await press(nodes.toPill, PRESS_HOLD_MS, gen);
      void moveCursor(1060, 640, 700, gen).catch((error: unknown) => {
        if (error !== STOPPED) {
          throw error;
        }
      });
      await morph("pill");
      await sleep(700, gen);
      roll("2");
      // UID 2: same tick as the bump, as the mock does (4a:1324).
      options.onBuilderAsks?.(true);
      await sleep(1300, gen);
      // UID 3: aim at the whole "N waiting" pill, as the mock's centerOf(.wt).
      const waitAt = center(nodes.count.closest<HTMLElement>(".wt") ?? nodes.count);
      await moveCursor(waitAt.x, waitAt.y, 620, gen);
      await sleep(220, gen);
      await press(nodes.composer, 120, gen);
      void moveCursor(1180, 470, 600, gen).catch((error: unknown) => {
        if (error !== STOPPED) {
          throw error;
        }
      });
      await morph("full");
      await sleep(500, gen);
      await showCursor(false, gen);
      await sleep(300, gen);
    } catch (error) {
      if (error !== STOPPED) {
        throw error;
      }
    } finally {
      if (generation === gen) {
        running = false;
      }
    }
  }

  paint("full");
  if (nodes.cursor) {
    nodes.cursor.style.opacity = "0";
    placeCursor(1120, 470);
  }

  return {
    form: () => form,
    snap: (next) => {
      generation += 1;
      running = false;
      clock.cancelAll();
      paint(next);
      if (native) {
        void planNative(next).then((plan) => {
          if (!plan || form !== next) {
            return;
          }
          holdNative({ w: plan.target.width, h: plan.target.height });
          paint(next);
          return tweenNative(next, 0, plan).then(() => {
            if (form === next) {
              paint(next);
            }
          });
        });
      }
    },
    morph: (next) => morph(next),
    layout: () => {
      if (tween) {
        // A resize landed mid-morph: lay the in-flight frame into the new size.
        placeTween();
        return;
      }
      if (running) {
        return;
      }
      paint(form);
    },
    startSequence: () => {
      generation += 1;
      clock.cancelAll();
      paint("full");
      nodes.count.textContent = "1";
      options.onBuilderAsks?.(false);
      if (nodes.cursor) {
        nodes.cursor.style.opacity = "0";
      }
      void sequence(generation);
    },
    step: (dt) => clock.step(dt),
    now: () => clock.now(),
    get running() {
      return running;
    },
    get pending() {
      return clock.pending;
    },
    destroy: () => {
      generation += 1;
      clock.cancelAll();
    },
  };
}
