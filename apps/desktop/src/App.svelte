<script lang="ts">
  import { onMount, tick } from "svelte";
  import { flip, type AnimationConfig } from "svelte/animate";
  import { linear } from "svelte/easing";
  import type { TransitionConfig } from "svelte/transition";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import {
    decide,
    emitPush,
    formatStreamTime,
    formatUsd,
    getSnapshot,
    hlcMillis,
    isTextEntry,
    shortEventId,
    undo,
    type Approval,
    type Decision,
    type Snapshot,
  } from "./lib/api";

  let snapshot = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let deciding = $state(false);
  let primed = $state(false);

  const reviewer = $derived(snapshot?.agents.find((agent) => agent.id === "reviewer") ?? null);
  const oldestPending = $derived.by(() => {
    const rows = snapshot?.approvals.filter((approval) => approval.status === "pending") ?? [];
    return rows.reduce<Approval | null>((oldest, approval) => {
      if (!oldest || approval.created_at < oldest.created_at) {
        return approval;
      }
      return oldest;
    }, null);
  });
  const latestSettled = $derived.by(() => {
    const rows = snapshot?.approvals.filter((approval) => approval.status !== "pending") ?? [];
    return rows.reduce<Approval | null>((latest, approval) => {
      if (!latest || approval.created_at > latest.created_at) {
        return approval;
      }
      return latest;
    }, null);
  });
  const shown = $derived(oldestPending ?? latestSettled);
  const stream = $derived.by(() => {
    const events = snapshot?.events ?? [];
    const requestKey = shown ? `approval-requested:${shown.id}` : "";
    return [...events].reverse().map((event) => {
      const source = event.source.trim();
      const shortId = shortEventId(event.id);
      const millis = hlcMillis(event.hlc);
      const detail = `${event.hlc} · ${shortId}`;
      return {
        id: event.id,
        mark: source.charAt(0).toUpperCase() || "·",
        when: millis == null ? "—" : formatStreamTime(millis),
        iso: millis == null ? "" : new Date(millis).toISOString(),
        who: source ? `${source} · ${event.kind}` : event.kind,
        detail,
        request: requestKey !== "" && event.idempotency_key === requestKey,
      };
    });
  });
  const anchored = $derived(stream.some((row) => row.request));
  const undoable = $derived.by(() => {
    const now = Date.now();
    const rows =
      snapshot?.approvals.filter((approval) => {
        return (
          (approval.status === "approved" || approval.status === "denied") &&
          !approval.committed &&
          approval.undo_until != null &&
          approval.undo_until > now
        );
      }) ?? [];
    return rows.reduce<Approval | null>((latest, approval) => {
      if (!latest || (approval.decided_at ?? 0) > (latest.decided_at ?? 0)) {
        return approval;
      }
      return latest;
    }, null);
  });

  async function refresh(): Promise<void> {
    try {
      snapshot = await getSnapshot();
      error = null;
      if (!primed) {
        await tick();
        primed = true;
      }
    } catch (err) {
      error = err instanceof Error ? err.message : "The daemon is not reachable.";
    }
  }

  async function simulate(): Promise<void> {
    busy = true;
    try {
      await emitPush();
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : "The event was not accepted.";
    } finally {
      busy = false;
    }
  }

  async function ondecide(id: string, decision: Decision, reason?: string): Promise<boolean> {
    deciding = true;
    try {
      await decide(id, decision, reason);
      await refresh();
      return true;
    } catch (err) {
      error = err instanceof Error ? err.message : "The decision was not recorded.";
      return false;
    } finally {
      deciding = false;
    }
  }

  async function onundo(id: string): Promise<boolean> {
    deciding = true;
    try {
      await undo(id);
      await refresh();
      return true;
    } catch (err) {
      error = err instanceof Error ? err.message : "The decision could not be undone.";
      return false;
    } finally {
      deciding = false;
    }
  }

  function onWindowKey(event: KeyboardEvent): void {
    if (event.repeat || isTextEntry(event.target)) {
      return;
    }
    if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) {
      return;
    }
    if (event.key !== "z" && event.key !== "Z") {
      return;
    }
    const target = undoable;
    if (!target || target.undo_until == null || target.undo_until <= Date.now()) {
      return;
    }
    event.preventDefault();
    if (shown?.id === target.id) {
      return;
    }
    void onundo(target.id);
  }

  function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
    const cx = 3 * x1;
    const bx = 3 * (x2 - x1) - cx;
    const ax = 1 - cx - bx;
    const cy = 3 * y1;
    const by = 3 * (y2 - y1) - cy;
    const ay = 1 - cy - by;
    const sampleX = (t: number) => ((ax * t + bx) * t + cx) * t;
    const sampleY = (t: number) => ((ay * t + by) * t + cy) * t;
    const sampleDX = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
    const solveX = (x: number) => {
      let guess = x;
      for (let i = 0; i < 8; i += 1) {
        const error = sampleX(guess) - x;
        if (Math.abs(error) < 1e-6) {
          return guess;
        }
        const slope = sampleDX(guess);
        if (Math.abs(slope) < 1e-6) {
          break;
        }
        guess -= error / slope;
      }
      let lo = 0;
      let hi = 1;
      guess = x;
      for (let i = 0; i < 24; i += 1) {
        const xEst = sampleX(guess);
        if (Math.abs(xEst - x) < 1e-6) {
          return guess;
        }
        if (xEst < x) {
          lo = guess;
        } else {
          hi = guess;
        }
        guess = (lo + hi) / 2;
      }
      return guess;
    };
    return (x: number) => sampleY(solveX(x));
  }

  const easeOut = cubicBezier(0.22, 1, 0.36, 1);

  function tokenMs(name: string, fallback: number): number {
    const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    const value = Number.parseFloat(raw);
    return Number.isFinite(value) ? value : fallback;
  }

  function prefersReducedMotion(): boolean {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  }

  // New rows rise 8px. The motion reads --dur-soft / --ease-out at start time.
  // Reduced motion is a 160ms fade with no travel. The first paint does not play it.
  function arrive(_node: Element, params: { play: boolean }): TransitionConfig {
    if (!params.play) {
      return { duration: 0 };
    }
    if (prefersReducedMotion()) {
      return {
        duration: 160,
        easing: linear,
        css: (t) => `opacity: ${t};`,
      };
    }
    return {
      duration: tokenMs("--dur-soft", 360),
      easing: easeOut,
      css: (t, u) => `opacity: ${t}; transform: translateY(${u * 8}px);`,
    };
  }

  // Existing rows step to their new slots. Translate only: Svelte's flip also
  // scales, which would squash the row a receipt opens in.
  function stepRows(node: Element, coords: { from: DOMRect; to: DOMRect }): AnimationConfig {
    const dx = coords.from.left - coords.to.left;
    const dy = coords.from.top - coords.to.top;
    const moved = Math.abs(dx) >= 1 || Math.abs(dy) >= 1;
    const duration = prefersReducedMotion() || !moved ? 0 : tokenMs("--dur-soft", 360);
    const base = flip(node, coords, { duration, easing: easeOut });
    const css = base.css;
    if (!css || duration === 0) {
      return { duration: 0 };
    }
    return {
      ...base,
      duration,
      easing: easeOut,
      css: (t, u) => css(t, u).replace(/scale\([^)]*\);?$/, "scale(1, 1);"),
    };
  }

  function pinOverlay(stage: HTMLElement): () => void {
    const column = stage.closest("main");
    if (!column) {
      return () => {};
    }
    const place = () => {
      const box = column.getBoundingClientRect();
      stage.style.setProperty("--overlay-left", `${box.left}px`);
      stage.style.setProperty("--overlay-width", `${box.width}px`);
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(column);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, { passive: true });
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place);
    };
  }

  function flipSlot(node: HTMLElement): () => void {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    let first = node.getBoundingClientRect();
    let over = node.classList.contains("over");
    let flying = false;
    const remember = () => {
      if (flying) {
        return;
      }
      first = node.getBoundingClientRect();
    };
    const observer = new ResizeObserver(remember);
    observer.observe(node);
    const classes = new MutationObserver(() => {
      const nextOver = node.classList.contains("over");
      if (nextOver === over) {
        return;
      }
      const origin = first;
      over = nextOver;
      const last = node.getBoundingClientRect();
      const dx = origin.left - last.left;
      const dy = origin.top - last.top;
      if (motion.matches || (Math.abs(dx) < 1 && Math.abs(dy) < 1)) {
        first = last;
        if (!over) {
          node.scrollIntoView({ block: "nearest", behavior: motion.matches ? "auto" : "smooth" });
        }
        return;
      }
      flying = true;
      if (!nextOver) {
        node.style.zIndex = "4";
        node.style.position = "relative";
      }
      node.style.transition = "none";
      node.style.transform = `translate(${dx}px, ${dy}px)`;
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          node.style.transition = "transform var(--dur-stage) var(--ease-out)";
          node.style.transform = "translate(0px, 0px)";
        });
      });
      const done = (event: TransitionEvent) => {
        if (event.propertyName !== "transform") {
          return;
        }
        node.removeEventListener("transitionend", done);
        node.style.transition = "";
        node.style.transform = "";
        node.style.zIndex = "";
        node.style.position = "";
        flying = false;
        if (!over) {
          node.scrollIntoView({ block: "nearest", behavior: motion.matches ? "auto" : "smooth" });
        }
        first = node.getBoundingClientRect();
      };
      node.addEventListener("transitionend", done);
    });
    classes.observe(node, { attributes: true, attributeFilter: ["class"] });
    return () => {
      observer.disconnect();
      classes.disconnect();
    };
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<svelte:window onkeydown={onWindowKey} />

{#snippet approvalSlot()}
  <div class={["slot", shown?.status === "pending" && "over"]} {@attach flipSlot}>
    {#if shown}
      {#key shown.id}
        <ApprovalCard
          approval={shown}
          busy={deciding}
          shortcutTarget={shown.status === "pending"}
          ondecide={(decision, reason) => ondecide(shown.id, decision, reason)}
          onundo={() => onundo(shown.id)}
        />
      {/key}
    {:else}
      <div class="empty">
        <h2>Nothing is waiting.</h2>
        <p>A push wakes Reviewer. The turn drafts a comment and asks before any external effect. Approving records the decision and does not post it.</p>
      </div>
    {/if}
  </div>
{/snippet}

<div class="well">
  <div class="shell">
    <header class="titlebar">
      <div class="wordmark">
        <span class="dots" aria-hidden="true"></span>
        <strong>dasdevbot</strong>
      </div>
      <p class="status">
        {#if snapshot}
          {snapshot.role} · protocol {snapshot.protocol} · {snapshot.provider_detail} · sync {snapshot.sync}
        {:else}
          connecting
        {/if}
      </p>
    </header>

    {#if error}
      <p class="banner" role="alert">{error}</p>
    {/if}

    <div class="body">
      <aside>
        <p class="section">Agents</p>
        {#if reviewer}
          <div class="agent" data-status={reviewer.status}>
            <div class="agent-row">
              <h2>{reviewer.name}</h2>
              <span class="agent-status">{reviewer.status}</span>
            </div>
            <p class="project">{reviewer.project}</p>
            <p class="budget">{reviewer.tokens_spent} / {reviewer.token_cap} tok</p>
            <p class="persona">{reviewer.persona.trim()}</p>
            {#if reviewer.status === "working"}
              <div class="scan" aria-hidden="true"></div>
            {/if}
          </div>
        {:else}
          <p class="muted">No agents stored.</p>
        {/if}

        <button class="simulate" type="button" disabled={busy} onclick={() => void simulate()}>
          {busy ? "Waking Reviewer" : "Simulate repo.push"}
        </button>
        <p class="hint">Reviewer is a stored row. It runs only when this event wakes it.</p>
      </aside>

      <main>
        <p class="section">Stream</p>
        <div class="stage" {@attach pinOverlay}>
          <ol class="stream">
            {#each stream as row (row.id)}
              {@const filed = row.request && shown !== null && shown.status !== "pending"}
              <li class={filed ? "slot-row" : "event"} animate:stepRows>
                {#if !filed}
                  <div class="row" title={row.detail} in:arrive|global={{ play: primed }}>
                    <time class="when" datetime={row.iso || undefined}>{row.when}</time>
                    <span class="disc" aria-hidden="true">{row.mark}</span>
                    <div class="copy">
                      <p class="who">{row.who}</p>
                      <p class="detail">{row.detail}</p>
                    </div>
                  </div>
                {/if}
                {#if row.request && shown}
                  {@render approvalSlot()}
                {/if}
              </li>
            {/each}
          </ol>
          {#if !anchored}
            {@render approvalSlot()}
          {/if}
        </div>

        <p class="section ledger-head">Ledger</p>
        {#if snapshot && snapshot.ledger.length > 0}
          <ul class="ledger">
            {#each snapshot.ledger as line (line.id)}
              <li>
                {line.agent_name} · {line.project} · {line.provider} · {line.model} · {line.usage_kind} · {line.input_tokens} in / {line.output_tokens} out · {formatUsd(line.micro_usd)}
                <span>{line.note}</span>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="muted">No spend yet. Idle agents do not call a provider.</p>
        {/if}

      </main>
    </div>
  </div>
</div>

<style>
  .well {
    min-height: 100%;
    padding: var(--s-5);
    background: var(--paper-base);
  }

  .shell {
    max-width: 1120px;
    margin: 0 auto;
    background: var(--paper-raised);
    border: 1px solid var(--hairline);
    border-radius: var(--r-2xl);
    box-shadow: var(--shadow-float);
    overflow: hidden;
  }

  .titlebar {
    display: flex;
    justify-content: space-between;
    gap: var(--s-4);
    align-items: center;
    padding: var(--s-3) var(--s-4);
    border-bottom: 1px solid var(--hairline);
  }

  .wordmark {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  .dots {
    width: 42px;
    height: 10px;
    background:
      radial-gradient(circle at 5px 5px, var(--ink-3) 4px, transparent 4.5px),
      radial-gradient(circle at 21px 5px, var(--ink-3) 4px, transparent 4.5px),
      radial-gradient(circle at 37px 5px, var(--ink-3) 4px, transparent 4.5px);
  }

  .status,
  .budget,
  .ledger,
  .project {
    font-family: var(--font-machine);
  }

  .status {
    color: var(--ink-2);
    font-size: var(--t-micro);
    text-align: right;
  }

  .banner {
    margin: var(--s-3) var(--s-4) 0;
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-sm);
    color: var(--ink-1);
    background: var(--paper-sunken);
  }

  .body {
    display: grid;
    grid-template-columns: 280px 1fr;
    min-height: 640px;
  }

  aside {
    padding: var(--s-4);
    border-right: 1px solid var(--hairline);
  }

  main {
    padding: var(--s-4);
    background: var(--paper-base);
  }

  .section {
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
    margin-bottom: var(--s-3);
  }

  .agent {
    position: relative;
    overflow: hidden;
    padding: var(--s-3);
    border: 1px solid var(--hairline);
    border-radius: var(--r-lg);
    background: var(--paper-raised);
    box-shadow: var(--shadow-puff);
  }

  .agent-row {
    display: flex;
    justify-content: space-between;
    gap: var(--s-2);
    align-items: baseline;
  }

  h2 {
    font-size: var(--t-lead);
    line-height: var(--lh-lead);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  .agent-status,
  .project,
  .budget,
  .hint,
  .muted,
  .persona {
    color: var(--ink-2);
    font-size: var(--t-meta);
  }

  .persona {
    margin-top: var(--s-2);
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .budget {
    margin-top: var(--s-2);
    color: var(--ink-1);
  }

  .scan {
    margin-top: var(--s-3);
    height: 2px;
    overflow: hidden;
    background: var(--hairline);
  }

  .scan::after {
    content: "";
    display: block;
    height: 100%;
    width: 35%;
    background: var(--ink-1);
    animation: scan 1.1s linear infinite;
  }

  @media (prefers-reduced-motion: reduce) {
    .scan::after {
      animation: none;
    }
  }

  .simulate {
    width: 100%;
    margin-top: var(--s-4);
    height: 40px;
    border-radius: var(--r-md);
    border: 1.5px solid var(--ink-1);
    background: var(--paper-raised);
    color: var(--ink-1);
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    cursor: pointer;
  }

  .simulate:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .simulate:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .hint {
    margin-top: var(--s-2);
  }

  .stage {
    position: relative;
  }

  .stream {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0;
    margin: 0;
    padding: 0;
  }

  .event,
  .slot-row {
    list-style: none;
    border-radius: 0;
    background: none;
    box-shadow: none;
  }

  .row {
    display: grid;
    grid-template-columns: 76px 28px minmax(0, 1fr);
    column-gap: 12px;
    align-items: center;
    padding: 10px 2px 11px;
    border: 0;
    border-bottom: 1px solid var(--hairline);
    border-radius: 0;
    background: none;
    box-shadow: none;
  }

  .row:hover {
    border-bottom-color: var(--hairline-strong);
    background: color-mix(in oklab, var(--paper-sunken) 65%, transparent);
  }

  .when {
    font-family: var(--font-machine);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-medium);
    color: var(--ink-1);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .disc {
    width: 28px;
    height: 28px;
    flex: none;
    border-radius: var(--r-pill);
    background: var(--paper-sunken);
    box-shadow: 0 0 0 1px var(--hairline);
    display: grid;
    place-items: center;
    color: var(--ink-2);
    font-size: 12px;
    line-height: 1;
    font-weight: var(--w-semibold);
  }

  .copy {
    min-width: 0;
  }

  .who {
    color: var(--ink-1);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .detail {
    margin-top: 2px;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    overflow-wrap: anywhere;
  }

  /* Filed receipt: the card is the row. A hairline's padding or rule would sit under it. */
  .slot-row {
    padding: 0;
    border: 0;
    min-height: 0;
    height: auto;
  }

  .slot-row .slot {
    margin-top: 0;
  }

  .slot {
    margin-top: 14px;
    scroll-margin-bottom: 16px;
  }

  .slot.over {
    position: fixed;
    z-index: 4;
    left: var(--overlay-left, 0px);
    width: var(--overlay-width, 100%);
    bottom: 16px;
    display: flex;
    justify-content: center;
    margin-top: 0;
    padding: 0 28px;
    pointer-events: none;
    box-sizing: border-box;
  }

  .slot.over :global(article.card) {
    width: min(520px, 100%);
    pointer-events: auto;
  }

  .empty {
    padding: var(--s-5);
    border: 1px solid var(--hairline);
    border-radius: var(--r-lg);
    background: var(--paper-raised);
    box-shadow: var(--shadow-puff);
  }

  .empty h2 {
    font-size: var(--t-display);
    line-height: var(--lh-display);
  }

  .empty p,
  .muted {
    margin-top: var(--s-2);
    color: var(--ink-2);
  }

  .ledger-head {
    margin-top: var(--s-6);
  }

  .ledger {
    list-style: none;
    display: grid;
    gap: var(--s-2);
  }

  .ledger li {
    font-size: var(--t-meta);
    color: var(--ink-1);
  }

  .ledger span {
    display: block;
    color: var(--ink-2);
    font-size: var(--t-micro);
  }

  @media (max-width: 860px) {
    .well {
      padding: var(--s-2);
    }

    .body {
      grid-template-columns: 1fr;
    }

    aside {
      border-right: 0;
      border-bottom: 1px solid var(--hairline);
    }

    .titlebar {
      flex-direction: column;
      align-items: flex-start;
    }

    .status {
      text-align: left;
    }
  }
</style>
