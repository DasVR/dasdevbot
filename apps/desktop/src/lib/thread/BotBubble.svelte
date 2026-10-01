<script lang="ts">
  import { easeHeight } from "./motion";
  import RichText from "./RichText.svelte";

  interface Props {
    text: string;
    shown: string;
  }

  let { text, shown }: Props = $props();
</script>

<div class="bub">
  <p class="sizer" aria-hidden="true"><RichText {text} /></p>
  <div class="grow" {@attach easeHeight}>
    <p><RichText text={shown} /></p>
  </div>
</div>

<style>
  .bub {
    margin-left: var(--gutter);
    max-width: var(--bot-max);
    width: max-content;
    padding: 15px var(--s-5) var(--s-4);
    border-radius: var(--r-lg);
    background: var(--convex), var(--paper-raised);
  }

  .sizer {
    height: 0;
    overflow: hidden;
    visibility: hidden;
  }

  .grow {
    overflow: hidden;
  }

  p {
    text-wrap: pretty;
    white-space: pre-wrap;
  }

  .bub :global(code) {
    font-family: var(--font-machine);
    font-size: var(--t-meta);
    background: var(--paper-sunken);
    border-radius: var(--r-xs);
    padding: 1px 5px;
  }
</style>
