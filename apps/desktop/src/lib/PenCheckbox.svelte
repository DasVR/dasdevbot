<script lang="ts">
  import { CHECK_PATH } from "./pen";

  interface Props {
    label: string;
    checked: boolean;
    disabled?: boolean;
    draw?: boolean;
    note?: string | null;
    teammates: readonly string[];
    onToggle?: (next: boolean) => void;
  }

  let {
    label,
    checked,
    disabled = false,
    draw = false,
    note = null,
    teammates,
    onToggle,
  }: Props = $props();

  function onChange(event: Event): void {
    if (disabled) {
      return;
    }
    const input = event.currentTarget;
    if (!(input instanceof HTMLInputElement)) {
      return;
    }
    onToggle?.(input.checked);
  }
</script>

<label class={["rule", disabled && "locked"]}>
  <input type="checkbox" {checked} {disabled} onchange={onChange} />
  <span class="box" aria-hidden="true">
    <svg class="frame" viewBox="0 0 18 18">
      <rect x="1.25" y="1.25" width="15.5" height="15.5" rx="3" />
    </svg>
    {#if checked}
      <svg class="mark" viewBox="0 0 24 24">
        <path class={["path", draw && "draw"]} pathLength="1" d={CHECK_PATH} />
      </svg>
    {/if}
  </span>
  <span class="copy">
    <span class="name">{label}</span>
    {#if note}
      <span class="note">{note}</span>
    {/if}
    <span class="who">{teammates.join(", ")}</span>
  </span>
</label>

<style>
  .rule {
    position: relative;
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    column-gap: var(--s-3);
    align-items: start;
    padding: var(--s-3) 0;
    border-bottom: 1px solid var(--hairline);
    cursor: pointer;
  }

  .rule.locked {
    cursor: not-allowed;
  }

  .rule input {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: 0;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .rule:has(input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .box {
    position: relative;
    width: 18px;
    height: 18px;
    margin-top: 2px;
  }

  .frame {
    display: block;
    width: 18px;
    height: 18px;
    fill: none;
    stroke: var(--ink-1);
    stroke-width: 1.5px;
  }

  .locked .frame {
    stroke: var(--ink-3);
  }

  .mark {
    position: absolute;
    top: -4px;
    left: -1px;
    width: 24px;
    height: 24px;
    overflow: visible;
  }

  .path {
    fill: none;
    stroke: var(--pen);
    stroke-width: var(--pen-width);
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 1;
    stroke-dashoffset: 0;
  }

  .path.draw {
    stroke-dashoffset: 1;
    animation: draw-check var(--dur-draw) var(--ease-draw) 1 both;
  }

  .name {
    color: var(--ink-1);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .note {
    margin-left: var(--s-2);
    color: var(--ink-3);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .who {
    display: block;
    margin-top: 2px;
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  @keyframes draw-check {
    from {
      stroke-dashoffset: 1;
    }
    to {
      stroke-dashoffset: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .path.draw {
      animation: none;
      stroke-dashoffset: 0;
    }
  }
</style>
