<script lang="ts">
  import RichText from "./RichText.svelte";

  interface Props {
    text: string;
    time: string;
    hlc: string;
  }

  let { text, time, hlc }: Props = $props();
</script>

<div class="me">
  <div class="bub">
    <p><RichText {text} /></p>
  </div>
  {#if time}
    <time title={hlc || undefined}>{time}</time>
  {/if}
</div>

<style>
  .me {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--user-row-gap);
  }

  .bub {
    max-width: var(--user-max);
    padding: 10px var(--s-4) 11px;
    border-radius: var(--r-lg);
    background: var(--paper-sunken);
  }

  p {
    text-wrap: pretty;
  }

  time {
    padding: 0 var(--s-2);
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .bub :global(code) {
    font-family: var(--font-machine);
    font-size: var(--t-meta);
    background: color-mix(in oklab, var(--paper-base) 55%, var(--paper-sunken));
    border-radius: var(--r-xs);
    padding: 1px 5px;
  }
</style>
