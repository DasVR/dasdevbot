<script lang="ts">
  import { flip } from "svelte/animate";
  import type { AnimationConfig } from "svelte/animate";
  import { MediaQuery } from "svelte/reactivity";
  import type { TransitionConfig } from "svelte/transition";
  import { tokenEase, tokenMs } from "./cssTokens";
  import type { SheetRow } from "./queue";

  interface Props {
    rows: SheetRow[];
    done: SheetRow[];
    summary: string;
    daemonLine: string | null;
    onopen: (id: string) => void;
    onclose: () => void;
    onsettled?: () => void;
  }

  let { rows, done, summary, daemonLine, onopen, onclose, onsettled }: Props = $props();

  const reducedMotion = new MediaQuery("(prefers-reduced-motion: reduce)");

  function sheetMotion(
    _node: Element,
    _params: undefined,
    options: { direction: "in" | "out" | "both" },
  ): TransitionConfig {
    const intro = options.direction !== "out";
    const reduced = reducedMotion.current;
    const duration = intro ? tokenMs("--dur-stage", 520) : tokenMs("--dur-base", 240);
    const easing = intro ? tokenEase("--ease-out") : tokenEase("--ease-exit");
    if (reduced) {
      return { duration, easing: (t) => t, css: (t) => `opacity: ${t}` };
    }
    const travel = intro ? 12 : 8;
    return {
      duration,
      easing,
      css: (t, u) => `opacity: ${t}; transform: translateY(${u * travel}px)`,
    };
  }

  const titleId = "review-sheet-title";

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "Escape") {
      return;
    }
    event.preventDefault();
    onclose();
  }

  function shiftRows(node: Element, coords: { from: DOMRect; to: DOMRect }): AnimationConfig {
    const dx = coords.from.left - coords.to.left;
    const dy = coords.from.top - coords.to.top;
    const moved = Math.abs(dx) >= 1 || Math.abs(dy) >= 1;
    if (reducedMotion.current || !moved) {
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
</script>

<svelte:window onkeydown={onKeydown} />

<div
  class="sheet"
  role="dialog"
  aria-modal="true"
  aria-labelledby={titleId}
  tabindex="-1"
  data-review-sheet
  transition:sheetMotion|global
  onoutroend={() => onsettled?.()}
>
  <div class="sh">
    <svg class="gl" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="2.5" y="4.8" width="8.5" height="8.7" rx="1.6" />
      <path d="M5.2 2.5h6.7a1.6 1.6 0 0 1 1.6 1.6v6.8" />
    </svg>
    <h2 id={titleId}>Review</h2>
    <button class="close" type="button" onclick={onclose}>Close</button>
  </div>

  <p class="summary">{summary}</p>
  {#if daemonLine}
    <p class="daemon" data-daemon-line>{daemonLine}</p>
  {/if}

  {#if rows.length === 0 && done.length === 0}
    <p class="empty">Nothing is waiting on you.</p>
  {:else}
    {#if rows.length > 0}
      <h3>Waiting on you</h3>
      <div class="rows" data-queue-rows>
        {#each rows as row (row.id)}
          <div class="slot" animate:shiftRows data-created-at={row.createdAt}>
            {#if row.expired}
              <div class="rw" data-queue-row data-approval-id={row.id} data-expired="yes">
                <span class="dcol"><span class="xring" aria-hidden="true"></span></span>
                <span class="what">{row.title}</span>
              </div>
            {:else}
              <button
                class="rw"
                type="button"
                data-queue-row
                data-approval-id={row.id}
                data-agent-id={row.agentId}
                onclick={() => onopen(row.id)}
              >
                <span class="dcol">
                  <span class="wdot" data-waiting-dot aria-label="waiting on you"></span>
                </span>
                <span class="what"><b>{row.agent}</b> · {row.title}</span>
                <span class={["meta", row.expiring && "expiring"]}>{row.meta}</span>
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {:else}
      <p class="empty">Nothing is waiting on you.</p>
    {/if}

    {#if done.length > 0}
      <h3>Done</h3>
      <div class="rows" data-done-rows>
        {#each done as row (row.id)}
          <div class="rw" data-done-row data-approval-id={row.id}>
            <span class="dcol"><span class="xring" aria-hidden="true"></span></span>
            <span class="what">{row.title}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .sheet {
    position: fixed;
    z-index: 21;
    left: 0;
    right: 0;
    top: 92px;
    width: min(680px, calc(100% - 48px));
    margin: 0 auto;
    padding: 26px 32px 22px;
    background: var(--paper-raised);
    border: 1px solid var(--hairline);
    border-radius: var(--r-2xl);
    box-shadow: none;
  }

  .sheet:focus {
    outline: none;
  }

  .sheet:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .sh {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .gl {
    width: 18px;
    height: 18px;
    flex: none;
    color: var(--ink-1);
  }

  .gl rect,
  .gl path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  h2 {
    font-size: var(--t-title);
    line-height: var(--lh-title);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  .close {
    margin-left: auto;
    border: 0;
    background: none;
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: 1;
    font-weight: var(--w-medium);
    padding: 8px 10px;
    border-radius: var(--r-sm);
    cursor: pointer;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      transform var(--dur-base) var(--ease-out);
  }

  .close:hover {
    background: rgb(var(--shade) / 0.05);
  }

  .close:active {
    transform: scale(0.97, 0.955);
    transition-duration: var(--dur-fast);
    transition-timing-function: var(--ease-press);
  }

  .close:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .summary {
    margin-top: 8px;
    color: var(--ink-1);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .daemon,
  .empty {
    margin-top: 8px;
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  h3 {
    margin: 22px 0 6px;
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  .rows {
    border-top: 1px solid var(--hairline);
  }

  .slot {
    border-bottom: 1px solid var(--hairline);
  }

  .rw {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 50px;
    padding: 6px 2px;
    border: 0;
    border-radius: 0;
    background: none;
    box-shadow: none;
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
    font-weight: var(--w-regular);
    text-align: left;
    cursor: pointer;
  }

  div.rw {
    cursor: default;
  }

  .rw:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .what {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .what b {
    color: var(--ink-1);
    font-weight: var(--w-semibold);
  }

  .dcol {
    width: 14px;
    display: grid;
    place-items: center;
    flex: none;
  }

  .wdot {
    width: 7px;
    height: 7px;
    border-radius: var(--r-pill);
    background: var(--risk-external);
  }

  .xring {
    width: 8px;
    height: 8px;
    border-radius: var(--r-pill);
    box-shadow: inset 0 0 0 1.5px var(--warning);
  }

  .meta {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    font-weight: var(--w-regular);
    color: var(--ink-3);
    text-align: right;
    white-space: nowrap;
  }

  .meta.expiring {
    color: var(--ink-2);
  }

  @media (prefers-reduced-motion: reduce) {
    .close:hover,
    .rw:hover {
      transform: none;
    }
  }
</style>
