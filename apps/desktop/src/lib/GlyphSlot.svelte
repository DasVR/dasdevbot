<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    /** Fixed 48px frame. Curl stays in this box across steps. */
    glyph?: Snippet;
  }

  let { glyph }: Props = $props();
</script>

<span class="glyph-slot" data-glyph-slot data-size="48">
  {#if glyph}
    {@render glyph()}
  {/if}
</span>

<style>
  .glyph-slot {
    display: block;
    flex: none;
    width: 48px;
    height: 48px;
  }
</style>
