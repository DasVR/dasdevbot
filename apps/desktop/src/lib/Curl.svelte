<script lang="ts">
  import { onMount } from "svelte";
  import {
    BLINK_MS,
    CURL_DRAW_MS,
    EYE_DRAW_MS,
    curlDraws,
    curlMarks,
    noteCurlDraw,
  } from "./curlMark";

  interface Props {
    blink?: boolean;
    lookEpoch?: number;
    nodEpoch?: number;
    settleEpoch?: number;
    onLanded?: () => void;
  }

  let { blink = false, lookEpoch = 0, nodEpoch = 0, settleEpoch = 0, onLanded }: Props = $props();

  const marks = curlMarks();
  const reducedAtStart =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  let reduced = $state(reducedAtStart);
  let drawn = $state(reducedAtStart || curlDraws() > 0);
  let draws = $state(curlDraws());
  let blinks = $state(0);
  let blinkState = $state<"off" | "on" | "paused">("off");
  let lastLook = 0;
  let lastNod = 0;
  let lastSettle = 0;

  let svg = $state<SVGSVGElement | null>(null);
  let body = $state<SVGPathElement | null>(null);
  let eyeA = $state<SVGPathElement | null>(null);
  let eyeB = $state<SVGPathElement | null>(null);
  let pose = $state<SVGGElement | null>(null);
  let nod = $state<SVGGElement | null>(null);
  let look = $state<SVGGElement | null>(null);
  let blinker = $state<SVGGElement | null>(null);

  function motion(): boolean {
    return !reduced;
  }

  function drawPath(path: SVGPathElement, ms: number): Promise<void> {
    path.style.strokeDasharray = "1";
    path.style.strokeDashoffset = "1";
    const animation = path.animate([{ strokeDashoffset: "1" }, { strokeDashoffset: "0" }], {
      duration: ms,
      easing: "cubic-bezier(0.5, 0.05, 0.2, 1)",
      fill: "forwards",
    });
    return animation.finished.then(() => {
      path.style.strokeDashoffset = "0";
    });
  }

  async function drawOn(): Promise<void> {
    if (!body || !eyeA || !eyeB) {
      return;
    }
    noteCurlDraw();
    draws = curlDraws();
    await drawPath(body, CURL_DRAW_MS);
    await drawPath(eyeA, EYE_DRAW_MS);
    await drawPath(eyeB, EYE_DRAW_MS);
    drawn = true;
  }

  function showDrawn(): void {
    for (const path of [body, eyeA, eyeB]) {
      if (!path) {
        continue;
      }
      path.style.strokeDasharray = "1";
      path.style.strokeDashoffset = "0";
    }
    drawn = true;
  }

  function lookAtTitle(): void {
    if (!motion() || !svg || !pose || !look) {
      return;
    }
    const title = document.getElementById("step-title");
    if (!title) {
      return;
    }
    const box = svg.getBoundingClientRect();
    const aim = title.getBoundingClientRect();
    const dx = aim.left + aim.width / 2 - (box.left + box.width / 2);
    const dy = aim.top + aim.height / 2 - (box.top + box.height / 2);
    const length = Math.hypot(dx, dy) || 1;
    const tx = (dx / length) * 2.6;
    const ty = (dy / length) * 2.2;
    const rot = Math.max(-6, Math.min(6, (dx / length) * 6));
    const easing = "cubic-bezier(0.22, 1, 0.36, 1)";
    pose.animate([{ transform: "none" }, { transform: `rotate(${rot.toFixed(2)}deg)` }], {
      duration: 360,
      easing,
      fill: "forwards",
    });
    look.animate(
      [{ transform: "none" }, { transform: `translate(${tx.toFixed(2)}px, ${ty.toFixed(2)}px)` }],
      { duration: 360, easing, fill: "forwards" },
    );
  }

  function playBlink(): void {
    if (!motion() || !blinker) {
      return;
    }
    blinks += 1;
    blinker.animate(
      [
        { transform: "scaleY(1)" },
        { transform: "scaleY(0.1)", offset: 0.42 },
        { transform: "scaleY(0.1)", offset: 0.58 },
        { transform: "scaleY(1)" },
      ],
      { duration: 200, easing: "ease-in-out" },
    );
  }

  function playNod(): void {
    if (!motion() || !nod || !blinker) {
      return;
    }
    blinker.animate(
      [{ transform: "scaleY(1)" }, { transform: "scaleY(0.55)", offset: 0.4 }, { transform: "scaleY(1)" }],
      { duration: 520, easing: "cubic-bezier(0.65, 0, 0.35, 1)" },
    );
    nod.animate(
      [
        { transform: "none" },
        { transform: "translateY(2.4px) rotate(4deg) scaleY(0.96)", offset: 0.38 },
        { transform: "none" },
      ],
      { duration: 560, easing: "cubic-bezier(0.65, 0, 0.35, 1)" },
    );
  }

  function playSettle(): void {
    if (!motion() || !pose || !blinker) {
      return;
    }
    blinker.animate(
      [
        { transform: "scaleY(1)" },
        { transform: "scaleY(0.35)", offset: 0.45 },
        { transform: "scaleY(0.35)", offset: 0.65 },
        { transform: "scaleY(1)" },
      ],
      { duration: 900, easing: "cubic-bezier(0.65, 0, 0.35, 1)" },
    );
    pose.animate(
      [
        { transform: "none" },
        { transform: "translateY(2.2px) scaleY(0.93)", offset: 0.45 },
        { transform: "translateY(0.9px) scaleY(0.975)" },
      ],
      { duration: 900, easing: "cubic-bezier(0.22, 1, 0.36, 1)", fill: "forwards" },
    );
  }

  function traceRunning(): boolean {
    return document.querySelector("[data-running-trace]") != null;
  }

  $effect(() => {
    const epoch = lookEpoch;
    const ready = drawn;
    if (!ready || epoch === 0 || epoch === lastLook) {
      return;
    }
    lastLook = epoch;
    if (!motion()) {
      return;
    }
    lookAtTitle();
  });

  $effect(() => {
    const epoch = nodEpoch;
    const ready = drawn;
    if (!ready || epoch === 0 || epoch === lastNod) {
      return;
    }
    lastNod = epoch;
    if (!motion()) {
      return;
    }
    playNod();
  });

  $effect(() => {
    const epoch = settleEpoch;
    const ready = drawn;
    if (!ready || epoch === 0 || epoch === lastSettle) {
      return;
    }
    lastSettle = epoch;
    if (!motion()) {
      return;
    }
    playSettle();
  });

  onMount(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onMedia = () => {
      reduced = media.matches;
    };
    media.addEventListener("change", onMedia);
    reduced = media.matches;

    let timer = 0;
    let stopped = false;
    const arm = () => {
      window.clearTimeout(timer);
      if (stopped || !blink || reduced) {
        blinkState = "off";
        return;
      }
      if (traceRunning()) {
        blinkState = "paused";
        return;
      }
      blinkState = "on";
      timer = window.setTimeout(() => {
        if (traceRunning() || reduced) {
          arm();
          return;
        }
        playBlink();
        arm();
      }, BLINK_MS);
    };

    const observer = new MutationObserver(arm);
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["data-running-trace"],
    });

    const landed = () => {
      onLanded?.();
    };
    if (reduced || curlDraws() > 0) {
      showDrawn();
      landed();
      arm();
    } else {
      void drawOn().then(() => {
        landed();
        arm();
      });
    }

    return () => {
      stopped = true;
      window.clearTimeout(timer);
      observer.disconnect();
      media.removeEventListener("change", onMedia);
    };
  });
</script>

<svg
  bind:this={svg}
  class="curl"
  data-curl
  data-drawn={drawn ? "yes" : "no"}
  data-draws={draws}
  data-blinks={blinks}
  data-blink-ms={BLINK_MS}
  data-blink-state={blinkState}
  width="48"
  height="48"
  viewBox="0 0 48 48"
  aria-hidden="true"
>
  <g bind:this={pose} class="pose">
    <g bind:this={nod} class="nod">
      <path bind:this={body} class="pen body" pathLength="1" d={marks.body} />
      <g bind:this={look} class="look">
        <g bind:this={blinker} class="blink">
          <path bind:this={eyeA} class="pen eye" pathLength="1" d={marks.eyes[0]} />
          <path bind:this={eyeB} class="pen eye" pathLength="1" d={marks.eyes[1]} />
        </g>
      </g>
    </g>
  </g>
</svg>

<style>
  .curl {
    display: block;
    width: 48px;
    height: 48px;
    overflow: visible;
    flex: none;
  }

  .pose,
  .nod,
  .look,
  .blink {
    transform-box: view-box;
  }

  .pose,
  .nod {
    transform-origin: 24px 40px;
  }

  .look,
  .blink {
    transform-origin: 24px 25.7px;
  }

  .pen {
    fill: none;
    stroke: var(--ink-1);
    stroke-width: var(--pen-width);
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .curl[data-drawn="no"] .pen {
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
  }

  @media (prefers-reduced-motion: reduce) {
    .pose,
    .nod,
    .look,
    .blink {
      transform: none;
      animation: none;
    }
  }
</style>
