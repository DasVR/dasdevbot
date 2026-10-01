<script lang="ts">
  import { HAND_PATH } from "../pen";

  /** Pen weight. SVG stroke is unitless; the value matches --pen-width. */
  const PEN_STROKE = "1.75";

  interface Props {
    fromId: string;
    toId: string;
    label: string;
    drawn: boolean;
  }

  let { fromId, toId, label, drawn }: Props = $props();

  /** Prototype draws this pen at 420ms. --dur-draw is the check, not this mark. */
  const HAND_DRAW = "420ms";
  /** Lines start just after the row arrives. No duration token is 40. */
  const LINE_DELAY = "40ms";
  /** The pen waits for the lines to be underway. */
  const PEN_DELAY = "160ms";
  /** Ids fade in after the pen has started. */
  const ID_DELAY = "220ms";
</script>

<div
  class="handoff"
  role="note"
  style:--hand-draw={HAND_DRAW}
  style:--line-delay={LINE_DELAY}
  style:--pen-delay={PEN_DELAY}
  style:--id-delay={ID_DELAY}
>
  <span class="ln l"></span>
  <span class="mid">
    <svg viewBox="0 0 34 14" aria-hidden="true">
      <path
        class={["pen", drawn && "draw"]}
        pathLength="1"
        d={HAND_PATH}
        stroke-width={PEN_STROKE}
      />
    </svg>
    <span class="ids">{fromId} · <b>{label}</b> · {toId}</span>
  </span>
  <span class="ln r"></span>
</div>

<style>
  .handoff {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: 2px 0;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .ln {
    flex: 1;
    height: 1px;
    background: var(--hairline-strong);
    animation: grow var(--dur-stage) var(--ease-out) var(--line-delay) both;
  }

  .ln.l {
    transform-origin: right center;
  }

  .ln.r {
    transform-origin: left center;
  }

  .mid {
    display: flex;
    align-items: center;
    gap: 10px;
    white-space: nowrap;
  }

  svg {
    width: 34px;
    height: 14px;
    overflow: visible;
  }

  b {
    color: var(--ink-2);
    font-weight: var(--w-medium);
  }

  .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: 1.75;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 1 2;
    stroke-dashoffset: 1;
    vector-effect: non-scaling-stroke;
  }

  .draw {
    stroke-dashoffset: 0;
    animation: draw var(--hand-draw) var(--ease-draw) var(--pen-delay) both;
  }

  .ids {
    animation: id-in var(--dur-base) var(--ease-out) var(--id-delay) both;
  }

  @keyframes grow {
    from {
      transform: scaleX(0);
    }
  }

  @keyframes draw {
    from {
      stroke-dashoffset: 1;
    }
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  @keyframes id-in {
    from {
      opacity: 0;
      transform: translateY(3px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ln,
    .ids,
    .draw {
      animation: fade var(--dur-soft) linear both;
      transform: none;
    }

    .draw {
      stroke-dashoffset: 0;
    }
  }
</style>
