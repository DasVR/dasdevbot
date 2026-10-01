<script lang="ts">
  import { richParts } from "./rich";

  interface Props {
    text: string;
  }

  let { text }: Props = $props();

  const parts = $derived(richParts(text));
</script>

{#each parts as part, index (index)}
  {#if part.code}
    <code>{part.text}</code>
  {:else}
    {part.text}
  {/if}
{/each}
