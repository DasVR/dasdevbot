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
  isMockStage,
  placeComposer,
  placeWin,
} from "./geometry";
import { inTauri, prepareForm, readMetrics, setBounds, type NativeRect } from "./native";

const STOPPED = Symbol("stopped");

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

  function exactStage(): boolean {
    if (!options.stage) {
      return false;
    }
    const box = viewport();
    return isMockStage(box.w, box.h);
  }

  function winRect(next: ShellForm): ShellRect {
    const box = viewport();
    if (!options.stage) {
      return { x: 0, y: 0, w: box.w, h: box.h, r: MOCK_FORMS[next].win.r };
    }
    if (exactStage()) {
      return { ...MOCK_FORMS[next].win };
    }
    return placeWin(next, box);
  }

  function composerRect(next: ShellForm): ShellRect {
    const box = viewport();
    const win = winRect(next);
    if (exactStage()) {
      return { ...MOCK_FORMS[next].composer };
    }
    return placeComposer(next, box, win);
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

  function reserveCardClearance(next: ShellForm): void {
    const rect = composerRect(next);
    const box = viewport();
    const block = Math.max(0, box.h - rect.y);
    document.documentElement.style.setProperty("--composer-block", `${block}px`);
  }

  function paint(next: ShellForm): void {
    nodes.win.dataset.shellForm = next;
    syncLay();
    applyRect(nodes.win, winRect(next));
    applyRect(nodes.composer, composerRect(next));
    reserveCardClearance(next);
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
    concealStream(next !== "full");
    opacity(nodes.stream, pill ? 0 : 1);
    streamLayout(next);
    form = next;
    options.onForm(next);
  }

  function hits(next: ShellForm): void {
    nodes.layerFull.style.pointerEvents = next === "full" ? "auto" : "none";
    nodes.layerComp.style.pointerEvents = next === "companion" ? "auto" : "none";
    nodes.layerPill.style.pointerEvents = next === "pill" ? "auto" : "none";
  }

  function concealStream(collapsed: boolean): void {
    nodes.stream.inert = collapsed;
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
      void clock.play(outEl, [{ opacity: 1 }, { opacity: 0 }], 120, "linear", 0, "both");
      void clock.play(inEl, [{ opacity: 0 }, { opacity: 1 }], 120, "linear", 0, "both");
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

  async function tweenNative(next: ShellForm, ms: number): Promise<void> {
    if (!native) {
      return;
    }
    const prepared = prepareForm(next);
    const current = readMetrics();
    if (!prepared || !current) {
      return;
    }
    const target = await prepared;
    const from = await current;
    if (options.reduced() || ms <= 0) {
      await setBounds(next, target);
      return;
    }
    const ease = tokenEase("--ease-out");
    const t0 = performance.now();
    await new Promise<void>((resolve) => {
      const frame = () => {
        const p = Math.min(1, (performance.now() - t0) / ms);
        const e = ease(p);
        const rect: NativeRect = {
          x: from.x + (target.x - from.x) * e,
          y: from.y + (target.y - from.y) * e,
          width: from.width + (target.width - from.width) * e,
          height: from.height + (target.height - from.height) * e,
        };
        void setBounds(next, rect);
        if (p < 1) {
          window.requestAnimationFrame(frame);
        } else {
          resolve();
        }
      };
      window.requestAnimationFrame(frame);
    });
  }

  async function morph(next: ShellForm): Promise<void> {
    const from = form;
    if (from === next || animating) {
      return;
    }
    animating = true;
    nodes.win.dataset.shellForm = next;
    syncLay();
    reserveCardClearance(next);
    try {
    if (options.reduced()) {
      clock.cancelAll();
      paint(next);
      await tweenNative(next, 0);
      if (native && form === next) {
        paint(next);
      }
      return;
    }
    hits(next);
    if (next !== "full") {
      concealStream(true);
    }
    const fast = tokenMs("--dur-fast", 140);
    const soft = tokenMs("--dur-soft", 360);
    const base = tokenMs("--dur-base", 240);
    const stageMs = tokenMs("--dur-stage", 520);
    const outEase = easeCss("--ease-out");
    const fromWin = winRect(from);
    const toWin = winRect(next);
    const fromCmp = composerRect(from);
    const toCmp = composerRect(next);

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
    const motion = [
      clock.play(nodes.win, [rectFrames(fromWin), rectFrames(toWin)], stageMs, outEase),
      clock.play(nodes.composer, [rectFrames(fromCmp), rectFrames(toCmp)], stageMs, outEase),
      tweenNative(next, stageMs),
    ];
    await Promise.all(motion);
    if (native && form === next) {
      // The native window resizes while the tween runs, and commitStyles locked
      // the pre-resize rects in. Repaint against the settled viewport.
      paint(next);
    }
    } finally {
      animating = false;
      concealStream(form !== "full");
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

  function moveCursor(x: number, y: number, ms: number, gen: number): Promise<void> {
    if (options.reduced()) {
      return sleep(ms, gen).then(() => {
        placeCursor(x, y);
      });
    }
    const x0 = cursor.x;
    const y0 = cursor.y;
    const ease = tokenEase("--ease-in-out");
    return new Promise((resolve, reject) => {
      const t0 = clock.now();
      const frame = () => {
        if (generation !== gen) {
          reject(STOPPED);
          return;
        }
        const p = ms <= 0 ? 1 : Math.min(1, (clock.now() - t0) / ms);
        placeCursor(x0 + (x - x0) * p, y0 + (y - y0) * p);
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

  async function press(el: HTMLElement, ms: number, gen: number): Promise<void> {
    el.classList.add("is-press");
    if (nodes.cursor?.firstElementChild instanceof HTMLElement && !options.reduced()) {
      nodes.cursor.firstElementChild.style.scale = "0.92";
    }
    await sleep(ms, gen);
    el.classList.remove("is-press");
    if (nodes.cursor?.firstElementChild instanceof HTMLElement) {
      nodes.cursor.firstElementChild.style.scale = "";
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
      nodes.count.textContent = value;
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
      await sleep(1300, gen);
      const waitAt = center(nodes.count);
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
        void tweenNative(next, 0).then(() => {
          if (form === next) {
            paint(next);
          }
        });
      }
    },
    morph: (next) => morph(next),
    layout: () => {
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
      if (nodes.cursor) {
        nodes.cursor.style.opacity = "0";
      }
      void sequence(generation);
    },
    step: (dt) => clock.step(dt),
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
