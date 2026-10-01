<script lang="ts">
  import ApprovalCard from "../ApprovalCard.svelte";
  import type { Approval, Decision } from "../api";
  import BotBubble from "./BotBubble.svelte";
  import ToolSteps from "./ToolSteps.svelte";
  import type { AgentKey, TurnPart } from "./types";

  interface Props {
    agent: AgentKey;
    name: string;
    time: string;
    parts: TurnPart[];
    approval: Approval | null;
    animate?: boolean;
    busy?: boolean;
    ondecide?: (id: string, decision: Decision, reason?: string) => Promise<boolean>;
    onundo?: (id: string) => Promise<boolean>;
    onfile?: (id: string) => Promise<boolean>;
  }

  let { agent, name, time, parts, approval, animate = false, busy = false, ondecide, onundo, onfile }: Props =
    $props();

  const mark = $derived(agent === "builder" ? "B" : "R");

  function missed(part: never): string {
    return part;
  }
</script>

<section class="turn" data-agent={agent}>
  <div class="who">
    <span class="mg" aria-hidden="true">{mark}</span>
    <b>{name}</b>
    {#if time}<span>{time}</span>{/if}
  </div>
  {#each parts as part (part.type === "bubble" ? part.bubble.id : part.block.id)}
    {#if part.type === "bubble"}
      <BotBubble text={part.bubble.text} shown={part.bubble.shown} />
    {:else if part.type === "tools"}
      <ToolSteps block={part.block} {animate} />
    {:else}
      {missed(part)}
    {/if}
  {/each}
  {#if approval}
    <div class="slot">
      {#key approval.id}
        <ApprovalCard
          {approval}
          {busy}
          shortcutTarget={approval.status === "pending"}
          ondecide={(decision, reason) => ondecide?.(approval.id, decision, reason) ?? Promise.resolve(false)}
          onundo={() => onundo?.(approval.id) ?? Promise.resolve(false)}
          onfile={() => onfile?.(approval.id) ?? Promise.resolve(false)}
        />
      {/key}
    </div>
  {/if}
</section>

<style>
  .turn {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }

  .who {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    height: 20px;
  }

  .mg {
    width: var(--turn-mark);
    height: var(--turn-mark);
    border-radius: var(--r-pill);
    background: var(--convex), var(--paper-sunken);
    box-shadow: var(--highlight-top);
    display: grid;
    place-items: center;
    font-size: var(--t-micro);
    line-height: 1;
    font-weight: var(--w-regular);
    color: var(--ink-2);
  }

  b {
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .who span {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .slot {
    margin-left: var(--gutter);
    width: var(--slot-width);
    max-width: 100%;
  }

  .slot :global(article.card) {
    width: 100%;
  }
</style>
