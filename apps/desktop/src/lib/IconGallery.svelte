<script lang="ts">
  import FocusIcon from "./FocusIcon.svelte";
  import { CHECK_PATH, CHEVRON_PATH, DENY_MARK_PATH } from "./pen";

  /** Line icons. --pen-width is 1.75 and is only the pen. */
  const ICON_STROKE = "1.5";
  /** Check and deny are pen marks. Same weight as --pen-width. Not a new token. */
  const PEN_STROKE = "1.75";
</script>

<main style:--icon-stroke={ICON_STROKE} style:--pen-stroke={PEN_STROKE}>
  <h1>Line icons</h1>
  <p>Focus and the chevron are 1.5 line icons. Check and deny are pen marks at 1.75.</p>
  <ul>
    <li>
      <FocusIcon />
      <span>Focus</span>
    </li>
    <li>
      <svg class="line" viewBox="0 0 12 12" aria-hidden="true">
        <path d={CHEVRON_PATH} />
      </svg>
      <span>Chevron</span>
    </li>
    <li>
      <svg class="pen" viewBox="0 0 24 24" aria-hidden="true">
        <path d={CHECK_PATH} />
      </svg>
      <span>Check</span>
    </li>
    <li>
      <svg class="pen" viewBox="0 0 24 24" aria-hidden="true">
        <path d={DENY_MARK_PATH} />
      </svg>
      <span>Deny mark</span>
    </li>
  </ul>
</main>

<style>
  main {
    min-height: 100%;
    padding: var(--s-8);
    background: var(--paper-base);
    color: var(--ink-1);
    font-family: var(--font-ui);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  h1 {
    font-size: var(--t-title);
    line-height: var(--lh-title);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  p {
    margin-top: var(--s-2);
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  ul {
    list-style: none;
    display: flex;
    gap: var(--s-6);
    margin-top: var(--s-6);
    padding: var(--s-4);
    background: var(--paper-raised);
    border: 1px solid var(--hairline);
  }

  li {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-1);
    font-size: var(--t-meta);
  }

  svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  svg.line {
    stroke-width: var(--icon-stroke);
  }

  svg.pen {
    stroke-width: var(--pen-stroke);
  }
</style>
