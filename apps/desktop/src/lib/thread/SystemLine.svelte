<script lang="ts">
  interface Props {
    text: string;
    time: string;
    hlc: string;
    eventId: string;
  }

  let { text, time, hlc, eventId }: Props = $props();
</script>

<!-- The HLC is the title, so it shows on hover and stays off the line. -->
<p class="system" title={hlc || undefined}>
  {#if time}<time>{time}</time>{/if}
  <span>{text}</span>
  {#if eventId}<span class="id">{eventId}</span>{/if}
</p>

<style>
  .system {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin-left: var(--gutter);
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  time,
  .id {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .id {
    margin-left: auto;
  }
</style>
