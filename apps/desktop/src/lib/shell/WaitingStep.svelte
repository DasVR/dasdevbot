<script lang="ts">
  import { OPEN_WAITING_EVENT } from "./geometry";

  /**
   * The "waiting on you" step in the thread. It is the only way the main
   * window reaches a card: activating it asks the shell to show the full
   * form with this step focused, and inside Tauri the card window opens.
   * Nothing here decides anything.
   */
  interface Props {
    id: string;
    say: string;
    meta: string;
  }

  let { id, say, meta }: Props = $props();

  function open(event: MouseEvent): void {
    const node = event.currentTarget;
    if (node instanceof HTMLElement) {
      node.dispatchEvent(new CustomEvent(OPEN_WAITING_EVENT, { bubbles: true, detail: { id } }));
    }
  }
</script>

<button type="button" class="step" data-waiting={id} onclick={open}>
  <span class="mk" aria-hidden="true"><span class="wd"></span></span>
  <span class="say">{say}</span>
  <span class="el">waiting on you</span>
  <span class="dt">{meta}</span>
</button>

<style>
  .step {
    position: relative;
    display: grid;
    grid-template-columns: 28px 1fr auto;
    column-gap: 10px;
    align-items: start;
    width: 100%;
    padding: 5px 0 6px;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .step:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: var(--r-xs);
  }

  .mk {
    position: relative;
    width: 28px;
    height: 20px;
    display: grid;
    place-items: center;
  }

  .wd {
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
    font: var(--w-semibold) var(--t-meta) / 20px var(--font-ui);
    color: var(--ink-1);
  }

  .dt {
    grid-column: 2 / 4;
    font: var(--t-micro) / var(--lh-micro) var(--font-machine);
    color: var(--ink-3);
    padding-top: 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
