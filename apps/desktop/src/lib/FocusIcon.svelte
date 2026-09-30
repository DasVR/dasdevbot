<script lang="ts">
  import { FOCUS_INNER_PATH, FOCUS_OUTER_PATH } from "./pen";

  interface Props {
    label?: string;
  }

  let { label }: Props = $props();

  /** Line icon stroke. --pen-width is 1.75 and is only the pen. */
  const ICON_STROKE = "1.5";
</script>

<svg
  class="focus-glyph"
  viewBox="0 0 16 16"
  style:--icon-stroke={ICON_STROKE}
  aria-hidden={label ? undefined : "true"}
  aria-label={label}
>
  <path d={FOCUS_OUTER_PATH} />
  <path d={FOCUS_INNER_PATH} />
</svg>

<style>
  .focus-glyph {
    width: 16px;
    height: 16px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: var(--icon-stroke);
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
