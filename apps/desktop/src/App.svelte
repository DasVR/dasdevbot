<script lang="ts">
  import { onMount, tick } from "svelte";
  import SecretEntry from "./lib/SecretEntry.svelte";
  import ComposerBar from "./lib/thread/ComposerBar.svelte";
  import { emptyFrame, frameForShot, isShot } from "./lib/thread/fixture";
  import {
    BAR_HEIGHT,
    COMPOSER_INSET,
    GUTTER,
    MASK_FADE,
    SLOT_WIDTH,
    THREAD_GAP,
    THREAD_PAD_BOTTOM,
    THREAD_WIDTH,
    TOOLS_WIDTH,
    TURN_MARK,
    USER_BUBBLE_MAX,
    BOT_BUBBLE_MAX,
    DISC_RADIUS,
    DISC_SIZE,
    THREAD_PAD_TOP,
    USER_ROW_GAP,
  } from "./lib/thread/metrics";
  import { threadFromSnapshot, type LocalNote } from "./lib/thread/model";
  import { fileAsk, playStory, settleAsk, storyMarks, undoAsk } from "./lib/thread/play";
  import ThreadPane from "./lib/thread/ThreadPane.svelte";
  import type { ThreadFrame } from "./lib/thread/types";
  import {
    decide,
    emitPush,
    getSnapshot,
    undo,
    type Decision,
    type Snapshot,
  } from "./lib/api";

  const params = new URLSearchParams(window.location.search);
  const shot = isShot(params.get("shot")) ? params.get("shot") : null;
  const capture = params.has("capture");
  const autoplay = params.has("play");

  let frame = $state<ThreadFrame>(shot ? frameForShot(shot) : emptyFrame());
  let snapshot = $state<Snapshot | null>(null);
  let notes = $state<LocalNote[]>([]);
  let settingsOpen = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let primed = $state(false);
  let playing = $state(false);
  let generation = 0;

  const fixture = $derived(shot !== null || playing || capture);
  const project = $derived(snapshot?.agents.find((agent) => agent.id === "reviewer")?.project ?? "DasVR/NIL");

  function sleep(ms: number, gen: number): Promise<void> {
    return new Promise((resolve) => {
      window.setTimeout(() => resolve(), ms);
    });
  }

  async function refresh(): Promise<void> {
    if (shot || playing || capture) {
      return;
    }
    try {
      snapshot = await getSnapshot();
      error = null;
      frame.nodes = threadFromSnapshot(snapshot, notes);
      frame.chip = "Reviewer";
      frame.placeholder = "Message Reviewer";
      if (!primed) {
        await tick();
        primed = true;
      }
    } catch (err) {
      error = err instanceof Error ? err.message : "The daemon is not reachable.";
    }
  }

  async function simulate(forced: boolean): Promise<void> {
    busy = true;
    try {
      await emitPush(forced);
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : "The event was not accepted.";
    } finally {
      busy = false;
    }
  }

  async function ondecide(id: string, decision: Decision, reason?: string): Promise<boolean> {
    if (id === "ap_ask") {
      settleAsk(frame, decision);
      return true;
    }
    busy = true;
    try {
      await decide(id, decision, reason);
      await refresh();
      return true;
    } catch (err) {
      error = err instanceof Error ? err.message : "The decision was not recorded.";
      return false;
    } finally {
      busy = false;
    }
  }

  async function onfile(id: string): Promise<boolean> {
    if (id === "ap_ask") {
      fileAsk(frame);
      return true;
    }
    return true;
  }

  async function onundo(id: string): Promise<boolean> {
    if (id === "ap_ask") {
      undoAsk(frame);
      return true;
    }
    busy = true;
    try {
      await undo(id);
      await refresh();
      return true;
    } catch (err) {
      error = err instanceof Error ? err.message : "The decision could not be undone.";
      return false;
    } finally {
      busy = false;
    }
  }

  function ondraft(value: string): void {
    frame.draft = value;
  }

  function onsend(): void {
    const text = frame.draft.trim();
    if (!text || playing) {
      return;
    }
    frame.draft = "";
    if (shot || capture) {
      frame.nodes = [
        ...frame.nodes,
        { type: "user", id: `note-${crypto.randomUUID()}`, text, time: "", hlc: "" },
      ];
      return;
    }
    notes = [...notes, { id: `note-${crypto.randomUUID()}`, text, at: Date.now() }];
    if (snapshot) {
      frame.nodes = threadFromSnapshot(snapshot, notes);
    }
  }

  async function holdApprove(): Promise<void> {
    const card = document.querySelector('article.card[tabindex="0"]');
    if (!(card instanceof HTMLElement)) {
      return;
    }
    card.focus();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }));
    let hello: Element | null = null;
    for (let attempt = 0; attempt < 80 && !hello; attempt += 1) {
      hello = card.querySelector("button.hello");
      if (!hello) {
        await sleep(16.667, generation);
      }
    }
    window.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", ctrlKey: true, bubbles: true }));
    if (!(hello instanceof HTMLElement)) {
      return;
    }
    await sleep(600, generation);
    hello.click();
  }

  function placeComposerCursor(): void {
    const form = document.querySelector("form.composer");
    if (!(form instanceof HTMLElement)) {
      return;
    }
    const box = form.getBoundingClientRect();
    frame.sheenX = box.width * 0.72;
    frame.cursor = { x: box.left + box.width * 0.72, y: box.top + box.height * 0.45, visible: true };
    frame.lit = true;
  }

  async function replay(): Promise<void> {
    generation += 1;
    const gen = generation;
    playing = true;
    error = null;
    await playStory({
      frame,
      sleep: (ms) => sleep(ms, gen),
      stopped: () => gen !== generation,
      reduced: () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      holdApprove,
    });
    if (gen === generation) {
      playing = false;
    }
  }

  function syncSettings() {
    settingsOpen = globalThis.location.hash === "#settings";
  }

  onMount(() => {
    syncSettings();
    const onHash = () => syncSettings();
    globalThis.addEventListener("hashchange", onHash);
    document.documentElement.classList.toggle("cap", fixture);
    const root = window as Window & {
      __thread?: { start: () => void; readonly running: boolean; readonly marks: Record<string, number> };
    };
    root.__thread = {
      start: () => {
        void replay();
      },
      get running() {
        return playing;
      },
      get marks() {
        return storyMarks();
      },
    };
    if (shot === "composer") {
      void tick().then(placeComposerCursor);
    }
    if (autoplay || capture) {
      if (autoplay) {
        void replay();
      }
      return () => {
        globalThis.removeEventListener("hashchange", onHash);
        generation += 1;
      };
    }
    if (!shot) {
      void refresh();
      const timer = window.setInterval(() => {
        void refresh();
      }, 1000);
      return () => {
        globalThis.removeEventListener("hashchange", onHash);
        generation += 1;
        window.clearInterval(timer);
      };
    }
    return () => {
      globalThis.removeEventListener("hashchange", onHash);
      generation += 1;
    };
  });
</script>

<div
  class="shell"
  class:cap={fixture}
  style:--thread-width={THREAD_WIDTH}
  style:--gutter={GUTTER}
  style:--user-max={USER_BUBBLE_MAX}
  style:--bot-max={BOT_BUBBLE_MAX}
  style:--slot-width={SLOT_WIDTH}
  style:--tools-width={TOOLS_WIDTH}
  style:--bar-height={BAR_HEIGHT}
  style:--mask-fade={MASK_FADE}
  style:--composer-inset={COMPOSER_INSET}
  style:--thread-pad={THREAD_PAD_BOTTOM}
  style:--thread-pad-top={THREAD_PAD_TOP}
  style:--thread-gap={THREAD_GAP}
  style:--turn-mark={TURN_MARK}
  style:--user-row-gap={USER_ROW_GAP}
  style:--disc-radius={DISC_RADIUS}
  style:--disc-size={DISC_SIZE}
>
  <header class="top">
    <div class="wordmark">
      <span class="disc" aria-hidden="true">D</span>
      <b>dasdevbot</b>
      <span>{project} · phase0</span>
    </div>
    <div class="dev">
      <button type="button" onclick={() => void replay()}>Replay thread</button>
      <button type="button" disabled={busy} onclick={() => void simulate(false)}>
        {busy ? "Waking Reviewer" : "Simulate repo.push"}
      </button>
      <button type="button" disabled={busy} onclick={() => void simulate(true)}>
        {busy ? "Waking Reviewer" : "Simulate force push"}
      </button>
    </div>
  </header>

  {#if error && !fixture}
    <p class="banner" role="alert">{error}</p>
  {/if}

  <ThreadPane
    nodes={frame.nodes}
    animate={playing || (primed && !shot)}
    {busy}
    {ondecide}
    {onundo}
    {onfile}
  />

  <div class="dock">
    <ComposerBar
      draft={frame.draft}
      chip={frame.chip}
      placeholder={frame.placeholder}
      lit={frame.lit}
      sheenX={frame.sheenX}
      {ondraft}
      {onsend}
    />
  </div>

  {#if frame.cursor}
    <div
      class="cursor"
      class:on={frame.cursor.visible}
      style:left="{frame.cursor.x}px"
      style:top="{frame.cursor.y}px"
      aria-hidden="true"
    >
      <svg viewBox="0 0 20 20">
        <path d="M3 2.2 3.4 16l3.7-3.6 2.6 5.6 2.4-1.1-2.6-5.5 5.2-.3z" />
      </svg>
    </div>
  {/if}
  {#if settingsOpen}
    <SecretEntry />
  {/if}
</div>

<style>
  .shell {
    --grain: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='240' height='240'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='1.15' numOctaves='2' stitchTiles='stitch'/%3E%3CfeColorMatrix values='0 0 0 0 .227 0 0 0 0 .157 0 0 0 0 .086 .42 0 0 0 -.17'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
    height: 100vh;
    position: relative;
    overflow: hidden;
    background: var(--grain), var(--paper-base);
    color: var(--ink-1);
  }

  .top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: var(--bar-height);
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: 0 var(--s-6);
    z-index: 5;
  }

  .wordmark {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .disc {
    width: 26px;
    height: 26px;
    border-radius: var(--disc-radius);
    background: var(--ink-1);
    color: var(--paper-raised);
    display: grid;
    place-items: center;
    font-size: var(--disc-size);
    font-weight: var(--w-semibold);
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.16);
  }

  .wordmark b {
    letter-spacing: var(--track-tight);
  }

  .wordmark span {
    color: var(--ink-3);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .dev {
    margin-left: auto;
    display: flex;
    gap: var(--s-2);
  }

  .cap .dev {
    display: none;
  }

  .dev button {
    height: 30px;
    padding: 0 var(--s-3);
    border-radius: var(--r-pill);
    border: 1px solid var(--hairline-strong);
    background: var(--paper-raised);
    color: var(--ink-1);
    font-size: var(--t-meta);
    font-weight: var(--w-medium);
    cursor: pointer;
  }

  .dev button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .banner {
    position: absolute;
    top: var(--bar-height);
    left: var(--s-6);
    right: var(--s-6);
    z-index: 4;
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-sm);
    background: var(--paper-sunken);
    color: var(--ink-1);
  }

  .dock {
    position: fixed;
    left: 0;
    right: 0;
    bottom: var(--composer-inset);
    width: var(--thread-width);
    margin: 0 auto;
    z-index: 6;
  }

  .cursor {
    position: fixed;
    z-index: 8;
    width: 20px;
    height: 20px;
    margin: -2px 0 0 -3px;
    pointer-events: none;
    color: var(--ink-1);
    opacity: 0;
    transition: opacity 160ms var(--ease-out);
  }

  .cursor.on {
    opacity: 1;
  }

  @media (prefers-reduced-motion: reduce) {
    .cursor {
      transition: opacity 120ms linear;
    }
  }

  .cursor svg {
    width: 20px;
    height: 20px;
    overflow: visible;
  }

  .cursor path {
    fill: var(--ink-1);
    stroke: var(--paper-raised);
    stroke-width: 1.3;
    stroke-linejoin: round;
  }
</style>
