<script lang="ts">
  import { INK_PRESS } from "./metrics";

  interface Props {
    draft: string;
    chip: string;
    placeholder: string;
    lit: boolean;
    sheenX: number;
    ondraft?: (value: string) => void;
    onsend?: () => void;
  }

  let { draft, chip, placeholder, lit, sheenX, ondraft, onsend }: Props = $props();

  let hovered = $state(false);
  let focused = $state(false);
  const glass = $derived(lit || hovered || focused);
  const canSend = $derived(draft.trim().length > 0);

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (canSend) {
        onsend?.();
      }
    }
  }

  function onPointer(event: PointerEvent): void {
    const form = event.currentTarget;
    if (!(form instanceof HTMLElement)) {
      return;
    }
    const box = form.getBoundingClientRect();
    form.style.setProperty("--sheen-x", `${event.clientX - box.left}px`);
  }
</script>

<form
  class={["composer", glass && "lit"]}
  style:--ink-press={INK_PRESS}
  style:--sheen-x="{sheenX}px"
  autocomplete="off"
  onsubmit={(event) => {
    event.preventDefault();
    if (canSend) {
      onsend?.();
    }
  }}
  onpointerenter={() => (hovered = true)}
  onpointerleave={() => (hovered = false)}
  onpointermove={onPointer}
  onfocusin={() => (focused = true)}
  onfocusout={() => (focused = false)}
>
  <div class="grain" aria-hidden="true"></div>
  <div class="rim" aria-hidden="true"></div>
  <div class="toplight" aria-hidden="true"></div>
  <div class="spec" aria-hidden="true"><i></i></div>
  <div class="sheen" aria-hidden="true"><i></i></div>
  <button class="cbtn plus" type="button" aria-label="Attach" disabled>
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M8 3.2v9.6M3.2 8h9.6" />
    </svg>
  </button>
  <span class="chip">@{chip}</span>
  <textarea
    rows="1"
    aria-label="Message"
    {placeholder}
    value={draft}
    oninput={(event) => ondraft?.(event.currentTarget.value)}
    onkeydown={onKeydown}
  ></textarea>
  <button class="cbtn send" type="submit" aria-label="Send" disabled={!canSend}>
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M8 13V3.5M3.8 7.4 8 3.2l4.2 4.2" />
    </svg>
  </button>
</form>

<style>
  .composer {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--bar-height);
    padding: 0 10px;
    border-radius: var(--r-2xl);
    background: var(--glass-fill);
    -webkit-backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    box-shadow: var(--glass-edge), var(--shadow-press), var(--shadow-float);
  }

  .composer.lit {
    background: var(--glass-fill);
  }

  .composer:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .grain,
  .rim,
  .toplight,
  .spec,
  .sheen {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    pointer-events: none;
  }

  .grain {
    opacity: 0.55;
    background-image: var(--grain);
    background-size: 254px 254px;
  }

  .rim,
  .toplight,
  .spec,
  .sheen {
    opacity: 0;
  }

  .composer.lit .rim,
  .composer.lit .toplight {
    opacity: 1;
  }

  .rim {
    padding: 1.5px;
    background: linear-gradient(
      180deg,
      rgb(255 255 255 / 1),
      rgb(255 255 255 / 0.62) 16%,
      rgb(255 255 255 / 0.12) 46%,
      rgb(255 255 255 / 0) 62%
    );
    -webkit-mask: linear-gradient(var(--ink-1) 0 0) content-box, linear-gradient(var(--ink-1) 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
  }

  .toplight {
    left: 22px;
    right: 22px;
    bottom: auto;
    height: 1px;
    background: linear-gradient(90deg, transparent, rgb(255 255 255 / 0.9), transparent);
  }

  .spec,
  .sheen {
    transition: opacity var(--dur-base) var(--ease-out);
  }

  .composer.lit .spec,
  .composer.lit .sheen {
    opacity: 1;
  }

  .spec {
    padding: 2px;
    -webkit-mask: linear-gradient(var(--ink-1) 0 0) content-box, linear-gradient(var(--ink-1) 0 0);
    -webkit-mask-composite: xor;
    mask-composite: exclude;
  }

  .spec i,
  .sheen i {
    position: absolute;
    left: var(--sheen-x);
    top: 0;
    border-radius: var(--r-pill);
    transform: translateX(-50%);
  }

  .spec i {
    width: 180px;
    height: 12px;
    background: rgb(255 255 255 / 0.85);
  }

  .sheen i {
    width: 260px;
    height: 36px;
    top: -8px;
    background: radial-gradient(closest-side, rgb(255 255 255 / 0.75), rgb(255 255 255 / 0));
  }

  .cbtn,
  .chip,
  textarea {
    position: relative;
    z-index: 1;
  }

  .cbtn {
    width: 40px;
    height: 40px;
    flex: none;
    border: 0;
    border-radius: 50%;
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .cbtn svg {
    width: 16px;
    height: 16px;
  }

  .cbtn path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .plus {
    background: var(--convex), var(--paper-raised);
    color: var(--ink-1);
    box-shadow: var(--highlight-top), 0 0 0 1px var(--hairline);
  }

  .plus:disabled {
    cursor: default;
  }

  .send {
    background: var(--ink-1);
    color: var(--paper-raised);
    box-shadow: var(--highlight-top), var(--shadow-puff);
  }

  .send:disabled {
    opacity: 0.45;
    cursor: default;
    box-shadow: none;
  }

  .send:active:not(:disabled) {
    background: var(--ink-press);
  }

  .chip {
    height: 30px;
    flex: none;
    display: inline-flex;
    align-items: center;
    padding: 0 11px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--paper-sunken) 94%, transparent);
    color: var(--ink-1);
    font-size: var(--t-meta);
    line-height: var(--t-meta);
    font-weight: var(--w-semibold);
  }

  textarea {
    flex: 1;
    min-width: 0;
    height: var(--lh-body);
    border: 0;
    resize: none;
    outline: none;
    padding: 0;
    background: none;
    font-family: var(--font-ui);
    font-size: var(--t-body);
    line-height: var(--lh-body);
    white-space: nowrap;
    overflow: hidden;
  }

  textarea::placeholder {
    color: var(--ink-2);
  }

  @media (prefers-reduced-motion: reduce) {
    .spec,
    .sheen {
      display: none;
    }

    .send:active:not(:disabled) {
      transform: none;
    }
  }

  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    .composer.lit {
      background: var(--glass-fill-solid);
    }
  }
</style>
