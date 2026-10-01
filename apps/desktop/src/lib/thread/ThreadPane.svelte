<script lang="ts">
  import { QUIET_LINE_PATH } from "../pen";
  import type { Decision } from "../api";
  import BotTurn from "./BotTurn.svelte";
  import DenyRow from "./DenyRow.svelte";
  import HandoffLine from "./HandoffLine.svelte";
  import { arrive, stepRows } from "./motion";
  import SystemLine from "./SystemLine.svelte";
  import type { ThreadNode } from "./types";
  import UserBubble from "./UserBubble.svelte";

  /** Pen weight for the empty-state stroke. Matches --pen-width. */
  const PEN_STROKE = "1.75";

  interface Props {
    nodes: ThreadNode[];
    animate?: boolean;
    busy?: boolean;
    ondecide?: (id: string, decision: Decision, reason?: string) => Promise<boolean>;
    onundo?: (id: string) => Promise<boolean>;
    onfile?: (id: string) => Promise<boolean>;
  }

  let { nodes, animate = false, busy = false, ondecide, onundo, onfile }: Props = $props();

  function missed(node: never): string {
    return node;
  }
</script>

<div class="viewport">
  <div class="thread" role="log" aria-live="polite" aria-relevant="additions">
    {#if nodes.length === 0}
      <p class="quiet">
        <svg class="quiet-line" viewBox="0 0 104 10" aria-hidden="true">
          <path class="pen draw" pathLength="1" d={QUIET_LINE_PATH} stroke-width={PEN_STROKE} />
        </svg>
        Nothing is waiting.
      </p>
    {/if}
    {#each nodes as node (node.id)}
      <div class="item" animate:stepRows in:arrive={{ play: animate }}>
        {#if node.type === "day"}
          <p class="day">{node.label}</p>
        {:else if node.type === "user"}
          <UserBubble text={node.text} time={node.time} hlc={node.hlc} />
        {:else if node.type === "turn"}
          <BotTurn
            {animate}
            agent={node.agent}
            name={node.name}
            time={node.time}
            parts={node.parts}
            approval={node.approval}
            {busy}
            {ondecide}
            {onundo}
            {onfile}
          />
        {:else if node.type === "handoff"}
          <HandoffLine fromId={node.fromId} toId={node.toId} label={node.label} drawn={node.drawn} />
        {:else if node.type === "system"}
          <SystemLine text={node.text} time={node.time} hlc={node.hlc} eventId={node.eventId} />
        {:else if node.type === "deny"}
          <DenyRow
            text={node.text}
            command={node.command}
            time={node.time}
            hlc={node.hlc}
            eventId={node.eventId}
          />
        {:else}
          {missed(node)}
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .viewport {
    position: absolute;
    top: var(--bar-height);
    right: 0;
    bottom: 0;
    left: 0;
    overflow-x: hidden;
    overflow-y: auto;
    display: flex;
    flex-direction: column-reverse;
    scrollbar-width: none;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, var(--ink-1) var(--mask-fade));
    mask-image: linear-gradient(to bottom, transparent 0, var(--ink-1) var(--mask-fade));
  }

  .thread {
    width: var(--thread-width);
    margin: 0 auto;
    padding: var(--thread-pad-top) 0 var(--thread-pad);
    display: flex;
    flex-direction: column;
    gap: var(--thread-gap);
  }

  .item {
    display: flex;
    flex-direction: column;
    width: 100%;
  }

  .day {
    align-self: center;
    width: max-content;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .quiet {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .quiet-line {
    width: 104px;
    height: 10px;
    overflow: visible;
  }

  .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: 1.75;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }

  .draw {
    stroke-dasharray: 1;
    stroke-dashoffset: 0;
    animation: draw var(--dur-draw) var(--ease-draw) both;
  }

  @keyframes draw {
    from {
      stroke-dashoffset: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .draw {
      animation: none;
    }
  }
</style>
