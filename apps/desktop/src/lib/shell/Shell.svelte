<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import {
    CHROME_DOT,
    DESK_WASH,
    PAPER_GRAIN,
    ROSTER_WIDTH,
    SEND_REST_OPACITY,
    TITLEBAR_HEIGHT,
    TITLE_BUTTON_RADIUS,
    WINDOW_SHADOW,
    type ShellForm,
  } from "./geometry";
  import {
    CLOSE_PATH,
    COMPANION_PATH,
    ICON_STROKE,
    MAXIMIZE_PATH,
    MIC_ARC,
    MIC_RECT,
    MINIMIZE_PATH,
    PLUS_PATH,
    SEND_PATH,
  } from "./icons";
  import { type ShellCapApi, type ShellStageApi } from "./globals";
  import { createShellMotion, type ShellMotion } from "./motion";
  import { inTauri, windowCommand } from "./native";
  import {
    liveRoster,
    stageRoster,
    type LiveAgent,
    type ReviewerChrome,
    type RosterRow,
  } from "./roster";

  interface ReviewItem {
    id: string;
    agent: string;
    title: string;
  }

  interface Props {
    agents: LiveAgent[];
    project: string;
    phase: string;
    reviewer: ReviewerChrome | null;
    pending: ReviewItem[];
    busy: boolean;
    stage: boolean;
    capture: boolean;
    onSimulate?: (forced: boolean) => void;
    children: Snippet;
  }

  let {
    agents,
    project,
    phase,
    reviewer,
    pending,
    busy,
    stage,
    capture,
    onSimulate,
    children,
  }: Props = $props();

  let motion = $state<ShellMotion | null>(null);

  let deskEl = $state<HTMLElement | null>(null);
  let winEl = $state<HTMLElement | null>(null);
  let composerEl = $state<HTMLElement | null>(null);
  let rosterEl = $state<HTMLElement | null>(null);
  let streamEl = $state<HTMLElement | null>(null);
  let titleFullEl = $state<HTMLElement | null>(null);
  let titleCompEl = $state<HTMLElement | null>(null);
  let layerFullEl = $state<HTMLElement | null>(null);
  let layerCompEl = $state<HTMLElement | null>(null);
  let layerPillEl = $state<HTMLElement | null>(null);
  let countEl = $state<HTMLElement | null>(null);
  let cursorEl = $state<HTMLElement | null>(null);
  let toCompEl = $state<HTMLElement | null>(null);
  let toPillEl = $state<HTMLElement | null>(null);

  const native = inTauri();

  const waitingShown = $derived(stage && pending.length === 0 ? 1 : pending.length);
  const rows = $derived(stage ? stageRoster() : liveRoster(agents, reviewer, "reviewer"));

  function reduced(): boolean {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  }

  function go(next: ShellForm): void {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest(".composer")) {
      active.blur();
    }
    void motion?.morph(next);
  }

  function rowClass(row: RosterRow): string {
    return row.selected ? "rrow sel" : "rrow";
  }

  function hold(write: (el: HTMLElement | null) => void) {
    return (el: HTMLElement) => {
      write(el);
      return () => write(null);
    };
  }

  const keepDesk = hold((el) => (deskEl = el));
  const keepWin = hold((el) => (winEl = el));
  const keepComposer = hold((el) => (composerEl = el));
  const keepRoster = hold((el) => (rosterEl = el));
  const keepStream = hold((el) => (streamEl = el));
  const keepTitleFull = hold((el) => (titleFullEl = el));
  const keepTitleComp = hold((el) => (titleCompEl = el));
  const keepLayerFull = hold((el) => (layerFullEl = el));
  const keepLayerComp = hold((el) => (layerCompEl = el));
  const keepLayerPill = hold((el) => (layerPillEl = el));
  const keepCount = hold((el) => (countEl = el));
  const keepCursor = hold((el) => (cursorEl = el));
  const keepToComp = hold((el) => (toCompEl = el));
  const keepToPill = hold((el) => (toPillEl = el));

  onMount(() => {
    if (!winEl || !composerEl || !rosterEl || !streamEl || !titleFullEl || !titleCompEl) {
      return;
    }
    if (!layerFullEl || !layerCompEl || !layerPillEl || !countEl) {
      return;
    }
    if (!toCompEl || !toPillEl) {
      return;
    }
    if (native) {
      document.documentElement.classList.add("tauri-shell");
    }
    if (stage) {
      document.documentElement.classList.add("shell-stage");
    }
    const created = createShellMotion(
      {
        win: winEl,
        composer: composerEl,
        roster: rosterEl,
        stream: streamEl,
        titleFull: titleFullEl,
        titleComp: titleCompEl,
        layerFull: layerFullEl,
        layerComp: layerCompEl,
        layerPill: layerPillEl,
        count: countEl,
        cursor: cursorEl,
        toComp: toCompEl,
        toPill: toPillEl,
      },
      {
        stage,
        capture,
        reduced,
        onForm: () => {},
      },
    );
    motion = created;
    const stageApi: ShellStageApi = {
      snap: (next: ShellForm) => created.snap(next),
      morph: (next: ShellForm) => created.morph(next),
      start: () => created.startSequence(),
      form: () => created.form(),
    };
    if (stage) {
      window.__shellStage = stageApi;
    }
    if (capture) {
      const cap: ShellCapApi = {
        start: () => created.startSequence(),
        step: (dt: number) => created.step(dt),
        pending: () => created.pending,
        running: () => created.running,
      };
      window.__shellCap = cap;
    }
    const onResize = () => {
      created.layout();
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      created.destroy();
      delete window.__shellStage;
      delete window.__shellCap;
    };
  });
</script>

  <div
    {@attach keepDesk}
    class="desk"
    class:stage
    class:native
  style:--roster-width="{ROSTER_WIDTH}px"
  style:--titlebar-height="{TITLEBAR_HEIGHT}px"
  style:--title-radius={TITLE_BUTTON_RADIUS}
  style:--chrome-dot={CHROME_DOT}
  style:--desk-wash={DESK_WASH}
  style:--paper-grain={PAPER_GRAIN}
  style:--window-shadow={WINDOW_SHADOW}
  style:--icon-stroke={ICON_STROKE}
  style:--send-rest={SEND_REST_OPACITY}
>
  {#if stage}
    <div class="osbar">
      <b>dasdevbot</b>
      <span>File</span>
      <span>Edit</span>
      <span>View</span>
      <span class="clock">Wed 11:06 AM</span>
    </div>
    <div class="other" aria-hidden="true">
      <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
      <div class="frost"></div>
    </div>
  {/if}

  <div class="win" {@attach keepWin} role="application" aria-label="dasdevbot">
    <header class="tbar" data-tauri-drag-region>
      {#if native}
        <div class="captions">
          <button type="button" aria-label="Minimize" onclick={() => void windowCommand("window_minimize")}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d={MINIMIZE_PATH} /></svg>
          </button>
          <button
            type="button"
            aria-label="Maximize"
            onclick={() => void windowCommand("window_toggle_maximize")}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d={MAXIMIZE_PATH} /></svg>
          </button>
          <button type="button" aria-label="Close" onclick={() => void windowCommand("window_close")}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d={CLOSE_PATH} /></svg>
          </button>
        </div>
      {:else}
        <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
      {/if}

      <div class="wordmark">
        <span class="title-full" {@attach keepTitleFull}>
          Reviewer <span class="dim">· {project} · {phase}</span>
        </span>
        <span class="title-comp" {@attach keepTitleComp}>Reviewer</span>
      </div>

      <div class="ctl">
        <button
          class="shb"
          type="button"
          aria-label="Companion window"
          {@attach keepToComp}
          onclick={() => go("companion")}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <rect x="2" y="3" width="12" height="10" rx="2" />
            <path d={COMPANION_PATH} />
          </svg>
        </button>
        <button class="shb" type="button" aria-label="Float as a pill" {@attach keepToPill} onclick={() => go("pill")}>
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <rect x="1.8" y="6" width="12.4" height="5" rx="2.5" />
          </svg>
        </button>
      </div>
    </header>

    <div class="lay full">
    <aside class="roster" {@attach keepRoster} aria-label="Teammates">
      <h2>TEAMMATES</h2>
      {#each rows as row (row.id)}
        <div class={rowClass(row)}>
          <span class="mg" aria-hidden="true">{row.monogram}</span>
          <span class="nm">
            <b>{row.name}</b>
            {#if row.line}
              <span class="sub">{row.line}</span>
            {/if}
          </span>
          {#if row.kind === "waiting"}
            <span class="wdot" aria-label="waiting on you"></span>
          {/if}
        </div>
      {/each}
      {#if onSimulate}
        <div class="sim">
          <button type="button" disabled={busy} onclick={() => onSimulate?.(false)}>
            {busy ? "Waking Reviewer" : "Simulate repo.push"}
          </button>
          <button type="button" disabled={busy} onclick={() => onSimulate?.(true)}>
            {busy ? "Waking Reviewer" : "Simulate force push"}
          </button>
          <p>Reviewer is a stored row. It runs only when this event wakes it.</p>
        </div>
      {/if}
    </aside>
    </div>

    <div class="stream-slot" {@attach keepStream}>
      {@render children()}
    </div>

  </div>

  <div class="composer" {@attach keepComposer}>
    <div class="grain"></div>
    <div class="rim"></div>
    <div class="toplight"></div>
    <div class="layers">
      <div class="cl full" {@attach keepLayerFull}>
        <span class="cbtn plus" aria-hidden="true">
          <svg viewBox="0 0 16 16"><path d={PLUS_PATH} /></svg>
        </span>
        <span class="chip">@Reviewer</span>
        <span class="ph">Message Reviewer</span>
        <span class="cbtn mic" aria-hidden="true">
          <svg viewBox="0 0 18 18"><rect {...MIC_RECT} /><path d={MIC_ARC} /></svg>
        </span>
        <span class="cbtn send" aria-hidden="true">
          <svg viewBox="0 0 16 16"><path d={SEND_PATH} /></svg>
        </span>
      </div>
      <div class="cl comp" {@attach keepLayerComp}>
        <span class="cbtn plus" aria-hidden="true">
          <svg viewBox="0 0 16 16"><path d={PLUS_PATH} /></svg>
        </span>
        <span class="ph">Reply to Reviewer</span>
        <span class="cbtn mic" aria-hidden="true">
          <svg viewBox="0 0 18 18"><rect {...MIC_RECT} /><path d={MIC_ARC} /></svg>
        </span>
      </div>
      <div class="cl pill" {@attach keepLayerPill}>
        <span class="mic-glyph" aria-hidden="true">
          <svg viewBox="0 0 18 18"><rect {...MIC_RECT} /><path d={MIC_ARC} /></svg>
        </span>
        <span class="ph">Ask Reviewer…</span>
        <span class="div"></span>
        <button type="button" class="wt" onclick={() => go("full")}>
          <span class="n" {@attach keepCount}>{waitingShown}</span> waiting
        </button>
      </div>
    </div>
  </div>

  {#if capture}
    <div class="cursor" {@attach keepCursor} aria-hidden="true">
      <svg viewBox="0 0 20 20">
          <path
            d="M3 2.2 3.4 16l3.7-3.6 2.6 5.6 2.4-1.1-2.6-5.5 5.2-.3z"
            fill="var(--ink-1)"
            stroke="var(--paper-raised)"
            stroke-width="1.3"
            stroke-linejoin="round"
          />
      </svg>
    </div>
  {/if}
</div>

<style>
  .desk {
    position: fixed;
    inset: 0;
    overflow: hidden;
    background: var(--paper-base);
  }

  .desk.stage {
    background: var(--paper-grain), var(--desk-wash);
  }

  .desk.native {
    background: transparent;
  }

  .osbar {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 0 18px;
    background: color-mix(in oklab, var(--paper-base) 72%, transparent);
    box-shadow: 0 1px 0 rgb(var(--shade) / 0.08);
    font-size: var(--t-meta);
    font-weight: var(--w-medium);
    color: var(--ink-1);
  }

  .osbar b {
    font-weight: var(--w-semibold);
  }

  .osbar .clock {
    margin-left: auto;
    color: var(--ink-2);
    font-weight: var(--w-regular);
  }

  .other {
    position: absolute;
    overflow: hidden;
    background: var(--paper-base);
    border-radius: var(--r-sm);
    box-shadow: 0 0 0 1px rgb(var(--shade) / 0.1), 0 18px 40px -18px rgb(var(--shade) / 0.25);
  }

  .stage .other {
    /* Mock desk window. 12px is not a radius token. It is not scaled. */
    left: 170px;
    top: 120px;
    width: 1100px;
    height: 900px;
    border-radius: 12px;
  }

  .other .frost {
    position: absolute;
    inset: 36px 0 0;
    background: color-mix(in oklab, var(--paper-base) 50%, transparent);
  }

  .win {
    position: fixed;
    z-index: 5;
    left: 0;
    top: 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--paper-grain), var(--paper-base);
    box-shadow: var(--window-shadow);
  }

  /* Computed visibility, not a class check: collapsed forms cannot approve. */
  .win[data-shell-form="companion"] :global(article.card),
  .win[data-shell-form="pill"] :global(article.card) {
    visibility: hidden;
  }

  .tbar {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: var(--titlebar-height);
    display: flex;
    align-items: center;
    padding: 0 14px 0 16px;
    gap: 8px;
    z-index: 3;
  }

  .dots {
    display: flex;
    gap: 8px;
  }

  .dots i {
    width: 12px;
    height: 12px;
    border-radius: var(--r-pill);
    background: var(--chrome-dot);
    box-shadow: inset 0 0 0 0.5px rgb(var(--shade) / 0.18);
  }

  .captions {
    display: flex;
    margin-left: -8px;
  }

  .captions button {
    width: 46px;
    height: var(--titlebar-height);
    border: 0;
    background: none;
    color: var(--ink-2);
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .captions button:hover {
    background: rgb(var(--shade) / 0.06);
    color: var(--ink-1);
  }

  .captions svg,
  .shb svg,
  .cbtn svg,
  .mic-glyph svg {
    width: 16px;
    height: 16px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: var(--icon-stroke);
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .wordmark {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: var(--titlebar-height);
    display: grid;
    pointer-events: none;
    text-align: center;
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    line-height: var(--titlebar-height);
    color: var(--ink-1);
  }

  .title-full,
  .title-comp {
    grid-area: 1 / 1;
  }

  .title-comp {
    opacity: 0;
  }

  .dim {
    color: var(--ink-3);
    font-weight: var(--w-regular);
  }

  .sim button:active:not(:disabled) {
    transform: none;
    transition: none;
    background: var(--paper-sunken);
  }

  .ctl {
    margin-left: auto;
    display: flex;
    gap: 2px;
    position: relative;
    z-index: 1;
  }

  .shb {
    width: 30px;
    height: 30px;
    border-radius: var(--title-radius);
    border: 0;
    background: none;
    color: var(--ink-2);
    display: grid;
    place-items: center;
    cursor: pointer;
    transition:
      transform var(--dur-base) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }

  .shb:hover,
  .shb.is-hover {
    background: rgb(var(--shade) / 0.06);
    color: var(--ink-1);
  }

  .shb:active,
  .shb.is-press {
    transform: scale(0.93);
    transition-duration: var(--dur-fast);
    transition-timing-function: var(--ease-press);
  }

  .lay {
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    width: var(--lay-full, 1360px);
  }

  .roster {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: var(--roster-width);
    padding: 58px 12px 0;
    overflow: auto;
    /* Mock roster wash: paper-sunken RGB at 55% alpha, not a color-mix. */
    background: rgb(237 231 221 / 0.55);
    box-shadow: 1px 0 0 var(--hairline);
  }

  .roster h2 {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    font-weight: var(--w-semibold);
    /* Mock `.roster h5` tracking. Not a letter-spacing token. */
    letter-spacing: 0.02em;
    color: var(--ink-3);
    margin: 6px 10px 8px;
  }

  .rrow {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 48px;
    padding: 6px 10px;
    border-radius: var(--r-md);
  }

  .rrow.sel {
    background: rgb(var(--shade) / 0.055);
  }

  .mg {
    width: 28px;
    height: 28px;
    flex: none;
    border-radius: var(--r-pill);
    display: grid;
    place-items: center;
    background: var(--convex), var(--paper-sunken);
    box-shadow: var(--highlight-top);
    color: var(--ink-2);
    /* Mock `.rrow .mg` is 12px. The card monogram stays 13. */
    font-size: 12px;
    font-weight: var(--w-semibold);
    line-height: 1;
  }

  .nm {
    flex: 1;
    min-width: 0;
  }

  .nm b {
    display: block;
    font-size: var(--t-meta);
    line-height: 18px;
    font-weight: var(--w-semibold);
  }

  .sub {
    display: block;
    white-space: nowrap;
    overflow: visible;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    /* Mock `.rrow .nm span` is `var(--t-micro)/16px`. 16 is not a line-height token. */
    line-height: 16px;
    color: var(--ink-3);
  }

  .wdot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: var(--r-pill);
    background: var(--risk-external);
  }

  .sim {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    margin: var(--s-4) 10px 0;
  }

  .sim button {
    height: 40px;
    border-radius: var(--r-md);
    border: 1.5px solid var(--ink-1);
    background: var(--paper-raised);
    color: var(--ink-1);
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    cursor: pointer;
  }

  .sim button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .sim p {
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .stream-slot {
    position: absolute;
    top: var(--titlebar-height);
    bottom: 0;
    left: calc(100% - var(--lay-full, 1360px) + var(--roster-width));
    width: calc(var(--lay-full, 1360px) - var(--roster-width));
    overflow: auto;
  }

  .composer {
    position: fixed;
    z-index: 8;
    display: grid;
    margin: 0;
    padding: 0;
    border: 0;
    overflow: hidden;
    color: var(--ink-1);
    background: var(--glass-fill);
    -webkit-backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    box-shadow: var(--glass-edge), var(--shadow-press), var(--shadow-float);
  }

  .grain,
  .rim,
  .toplight {
    position: absolute;
    pointer-events: none;
  }

  .grain {
    inset: 0;
    border-radius: inherit;
    background: var(--paper-grain);
    background-size: 254px 254px;
    background-position: 7px 3px;
    opacity: 0.55;
  }

  .rim {
    inset: 0;
    border-radius: inherit;
    padding: 1.5px;
    background: linear-gradient(
      180deg,
      rgb(255 255 255 / 1) 0%,
      rgb(255 255 255 / 0.62) 16%,
      rgb(255 255 255 / 0.12) 46%,
      rgb(255 255 255 / 0) 62%,
      rgb(255 255 255 / 0.22) 100%
    );
    -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
  }

  .toplight {
    left: 22px;
    right: 22px;
    top: 0;
    height: 1px;
    z-index: 3;
    background: linear-gradient(
      90deg,
      rgb(255 255 255 / 0),
      rgb(255 255 255 / 1) 18%,
      rgb(255 255 255 / 1) 82%,
      rgb(255 255 255 / 0)
    );
  }

  .layers {
    position: relative;
    z-index: 1;
    display: grid;
    height: 100%;
  }

  .cl {
    grid-area: 1 / 1;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    min-width: 0;
  }

  .cl.comp,
  .cl.pill {
    opacity: 0;
    pointer-events: none;
  }

  .cl.pill {
    padding: 0 20px 0 16px;
    gap: 11px;
  }

  .ph {
    flex: 1;
    min-width: 0;
    color: var(--ink-2);
    white-space: nowrap;
    overflow: hidden;
  }

  .cbtn {
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: var(--r-pill);
    display: grid;
    place-items: center;
  }

  .cl.comp .cbtn {
    width: 36px;
    height: 36px;
  }

  .plus,
  .mic {
    background: var(--convex), var(--paper-raised);
    color: var(--ink-1);
    box-shadow: var(--highlight-top), 0 0 0 1px rgb(var(--shade) / 0.1), 0 1px 2px rgb(var(--shade) / 0.08);
  }

  .plus svg {
    /* Mock plus path stroke. Not the 1.5 icon token. */
    stroke-width: 1.6;
  }

  .send svg {
    /* Mock send path stroke. Not the 1.5 icon token. */
    stroke-width: 1.7;
  }

  .send {
    background: var(--ink-1);
    color: var(--paper-raised);
    opacity: var(--send-rest);
  }

  .chip {
    height: 30px;
    flex: none;
    display: inline-flex;
    align-items: center;
    padding: 0 11px;
    border-radius: var(--r-pill);
    background: var(--paper-sunken);
    color: var(--ink-1);
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    white-space: nowrap;
  }

  .mic-glyph {
    width: 18px;
    height: 18px;
    flex: none;
    color: var(--ink-1);
    display: grid;
    place-items: center;
  }

  .cbtn.mic svg {
    /* Mock `.mic svg` is 17px on an 18 viewBox, stroke 1.5. */
    width: 17px;
    height: 17px;
  }

  .mic-glyph svg {
    width: 18px;
    height: 18px;
  }

  .div {
    width: 1px;
    height: 22px;
    background: var(--hairline-strong);
    flex: none;
  }

  button.wt {
    border: 0;
    background: none;
    padding: 0;
    cursor: pointer;
    color: var(--ink-1);
    font: inherit;
    font-weight: var(--w-medium);
    white-space: nowrap;
  }

  .wt .n {
    display: inline-block;
    font-variant-numeric: tabular-nums;
  }

  .cursor {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 100;
    width: 20px;
    height: 20px;
    pointer-events: none;
    opacity: 0;
  }

  .cursor svg {
    width: 20px;
    height: 20px;
    overflow: visible;
  }

  @media (prefers-reduced-motion: reduce) {
    .shb:active,
    .shb.is-press {
      transform: none;
      background: var(--paper-sunken);
    }

  }

  :global(html.tauri-shell),
  :global(html.tauri-shell body) {
    background: transparent;
  }

  :global(html.shell-stage),
  :global(html.shell-stage body) {
    overflow: hidden;
    height: 100%;
  }

  @media (prefers-reduced-transparency: reduce) {
    .composer {
      background: var(--glass-fill-solid);
      -webkit-backdrop-filter: none;
      backdrop-filter: none;
    }
  }
</style>
