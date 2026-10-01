<script lang="ts">
  import { DENY_MARK_PATH } from "../pen";

  /** Pen weight. Same as --pen-width, named because SVG stroke is unitless. */
  const PEN_STROKE = "1.75";

  interface Props {
    text: string;
    command: string;
    time: string;
    hlc: string;
    eventId: string;
  }

  let { text, command, time, hlc, eventId }: Props = $props();
</script>

<div class="deny" title={hlc || undefined} role="status">
  <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
    <path class="pen draw" pathLength="1" d={DENY_MARK_PATH} stroke-width={PEN_STROKE} />
  </svg>
  <div class="what">
    <p>{text}</p>
    {#if command}
      <p class="cmd">{command}</p>
    {/if}
  </div>
  <p class="id">
    {#if time}{time}<br />{/if}{eventId}
  </p>
</div>

<style>
  .deny {
    display: flex;
    align-items: flex-start;
    gap: var(--s-3);
    margin-left: var(--gutter);
    width: var(--slot-width);
    padding: 10px 2px;
    border-radius: 0;
    background: none;
    box-shadow: none;
    color: var(--ink-1);
  }

  .mark {
    width: 24px;
    height: 24px;
    flex: none;
    overflow: visible;
  }

  .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: 1.75;
    stroke-linecap: round;
    stroke-linejoin: round;
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

  .what {
    flex: 1;
    min-width: 0;
  }

  .cmd {
    margin-top: 2px;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .id {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    text-align: right;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .draw {
      animation: fade var(--dur-base) linear both;
    }

    @keyframes fade {
      from {
        opacity: 0;
      }
    }
  }
</style>
