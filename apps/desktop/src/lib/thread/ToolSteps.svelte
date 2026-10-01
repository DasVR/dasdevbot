<script lang="ts">
  import { undoClock, undoSecondsLeft } from "../ApprovalCard.svelte";
  import { CHECK_PATH, CHEVRON_PATH, READING_PATH, WRITING_PATH } from "../pen";
  import { arrive, collapseHeight } from "./motion";
  import type { ToolBlockView, ToolStepView, TraceKind } from "./types";

  /** Pen weight inside the step mark. Matches --pen-width. */
  const PEN_STROKE = "1.75";
  /** Chevron. Not the pen. */
  const ICON_STROKE = "1.5";

  interface Props {
    block: ToolBlockView;
    animate?: boolean;
  }

  let { block, animate = false }: Props = $props();

  let opened = $state<boolean | null>(null);
  const expanded = $derived(opened ?? block.open);

  const done = $derived(block.steps.filter((step) => step.phase === "done"));
  const summary = $derived(summaryText(done));

  function tracePath(kind: TraceKind): string {
    switch (kind) {
      case "reading":
        return READING_PATH;
      case "writing":
        return WRITING_PATH;
      default: {
        const exhaustive: never = kind;
        return exhaustive;
      }
    }
  }

  function summaryText(steps: ToolStepView[]): { count: string; secs: string } {
    const total = steps.reduce((sum, step) => sum + parseElapsed(step.elapsed), 0);
    const count = steps.length;
    return {
      count: `Used ${count} tool${count === 1 ? "" : "s"}`,
      secs: `· ${Math.round(total)}s`,
    };
  }

  function parseElapsed(value: string): number {
    const match = /^(\d+(?:\.\d+)?)s$/.exec(value);
    return match ? Number(match[1]) : 0;
  }

  function toggle(): void {
    opened = !expanded;
  }
</script>

<div class="tools" style:--icon-stroke={ICON_STROKE}>
  {#if block.folded}
    <button class="sum" type="button" aria-expanded={expanded} onclick={toggle}>
      <svg class="chev" viewBox="0 0 12 12" aria-hidden="true">
        <path d={CHEVRON_PATH} />
      </svg>
      <span>{summary.count}</span>
      <span class="mono">{summary.secs}</span>
    </button>
  {/if}
  <div class="list">
    {#each block.steps as step (step.id)}
      {@render row(step)}
    {/each}
  </div>
</div>

{#snippet row(step: ToolStepView)}
  <div
    class="step"
    class:tuck={block.folded && !expanded && step.phase === "done"}
    in:arrive={{ y: 5, play: animate }}
  >
    <div class="clip">
    <div class="in">
      <span class="mk">
        {#if step.phase === "live"}
          <svg class={["tr", step.trace]} viewBox="0 0 44 12" aria-hidden="true">
            <path class="pen live" pathLength="1" d={tracePath(step.trace)} stroke-width={PEN_STROKE} />
          </svg>
        {:else if step.phase === "done"}
          <svg class="ck" viewBox="0 0 24 24" aria-hidden="true">
            <path class="pen draw" pathLength="1" d={CHECK_PATH} stroke-width={PEN_STROKE} />
          </svg>
        {:else}
          <span class="wait" aria-hidden="true"></span>
        {/if}
      </span>
      <span class="say">{step.say}</span>
      <span class={["el", step.phase === "waiting" && "need"]}
        >{step.undo
          ? `${step.undo.verb} · undo ${undoSecondsLeft(step.undo.until, undoClock.now)}s`
          : step.elapsed || "0.0s"}</span
      >
      {#if step.detail && step.phase !== "done"}
        <p class="dt" out:collapseHeight>{step.detail}</p>
      {/if}
    </div>
    </div>
  </div>
{/snippet}

<style>
  .tools {
    margin-left: var(--gutter);
    width: var(--tools-width);
    max-width: 100%;
  }

  .sum {
    height: 30px;
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    margin-left: -6px;
    padding: 0 10px 0 6px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    color: var(--ink-2);
    font-size: var(--t-meta);
    font-weight: var(--w-medium);
    cursor: pointer;
  }

  .sum:hover {
    background: color-mix(in oklab, var(--ink-1) 5%, transparent);
  }

  .chev {
    width: 12px;
    height: 12px;
    transition: transform var(--dur-base) var(--ease-out);
  }

  .sum[aria-expanded="true"] .chev {
    transform: rotate(90deg);
  }

  .chev path {
    fill: none;
    stroke: currentColor;
    stroke-width: var(--icon-stroke);
    vector-effect: non-scaling-stroke;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .mono {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    color: var(--ink-3);
  }

  .step {
    position: relative;
    display: grid;
    grid-template-rows: 1fr;
    transition: grid-template-rows var(--dur-soft) var(--ease-out);
  }

  .step.tuck {
    grid-template-rows: 0fr;
    transition-timing-function: var(--ease-in-out);
  }

  .step.tuck::after {
    display: none;
  }

  .clip {
    overflow: hidden;
    min-height: 0;
  }

  .in {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) auto;
    column-gap: 10px;
    padding: 5px 0 6px;
    align-items: start;
  }

  .mk {
    width: 22px;
    height: 20px;
    display: grid;
    place-items: center;
  }

  .tr,
  .ck {
    overflow: visible;
  }

  .tr {
    width: 22px;
    height: 8px;
  }

  .ck {
    width: 16px;
    height: 16px;
  }

  .pen {
    fill: none;
    stroke: var(--ink-2);
    stroke-width: 1.75;
    stroke-linecap: round;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }

  .ck .pen {
    stroke: var(--pen);
  }

  .live {
    stroke-dasharray: 1 2;
    animation: trace var(--dur-trace-read) linear infinite;
  }

  .tr.writing .live {
    animation-duration: var(--dur-trace-write);
  }

  .draw {
    stroke-dasharray: 1;
    stroke-dashoffset: 0;
    animation: draw var(--dur-draw) var(--ease-draw) 60ms both;
  }

  .wait {
    width: 7px;
    height: 7px;
    border-radius: var(--r-pill);
    background: var(--risk-external);
  }

  .say {
    color: var(--ink-2);
    line-height: 20px;
  }

  .el {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: 20px;
    color: var(--ink-3);
    font-variant-numeric: tabular-nums;
  }

  .el.need {
    font-family: var(--font-ui);
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    color: var(--ink-1);
  }

  .dt {
    grid-column: 2 / 4;
    padding-top: 2px;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .step::after {
    content: "";
    position: absolute;
    left: 10.5px;
    top: 25px;
    bottom: -3px;
    width: 1px;
    background: var(--hairline);
  }

  .step:last-child::after {
    display: none;
  }

  @keyframes trace {
    0% {
      stroke-dashoffset: 1;
      opacity: 1;
    }
    55% {
      stroke-dashoffset: 0;
      opacity: 1;
    }
    80% {
      stroke-dashoffset: 0;
      opacity: 1;
    }
    100% {
      stroke-dashoffset: 0;
      opacity: 0;
    }
  }

  @keyframes draw {
    from {
      stroke-dashoffset: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .step,
    .step.tuck,
    .chev,
    .sum[aria-expanded="true"] .chev {
      transition: none;
      transform: none;
    }

    .live {
      animation: none;
      stroke-dashoffset: 0;
      opacity: 1;
    }

    .draw {
      animation: fade var(--dur-base) linear 0s both;
    }

    @keyframes fade {
      from {
        opacity: 0;
      }
    }
  }
</style>
