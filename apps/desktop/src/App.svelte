<script lang="ts">
  import { onMount, tick } from "svelte";
  import { flip, type AnimationConfig } from "svelte/animate";
  import { linear } from "svelte/easing";
  import type { TransitionConfig } from "svelte/transition";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import SecretEntry from "./lib/SecretEntry.svelte";
  import { tokenEase, tokenMs } from "./lib/cssTokens";
  import { QUIET_LINE_PATH } from "./lib/pen";
  import { startPolling } from "./lib/poll";
  import { shellSearch } from "./lib/shell/geometry";
  import type { ReviewerChrome } from "./lib/shell/roster";
  import Shell from "./lib/shell/Shell.svelte";
  import StageThread from "./lib/shell/StageThread.svelte";
  import {
    decide,
    emitPush,
    formatStreamTime,
    formatUsd,
    getSnapshot,
    hlcMillis,
    openCardWindow,
    shortEventId,
    tauriWindowLabel,
    undo,
    type Approval,
    type Decision,
    type Snapshot,
  } from "./lib/api";

  /** Flat rows and the roster. tokens.css has no zero radius. */
  const FLAT_RADIUS = "0";
  /** The shell mock's phase label. The snapshot names a project, not a phase. */
  const PHASE = "phase0";
  const shell = typeof window === "undefined" ? { stage: false, capture: false } : shellSearch();
  /** Reduced-motion fades. tokens.css has no linear easing token. */
  const REDUCED_FADE_EASE = linear;

  let snapshot = $state<Snapshot | null>(null);
  let settingsOpen = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let deciding = $state(false);
  let primed = $state(false);
  let now = $state(Date.now());

  const STREAM_ROWS = 40;
  const reviewer = $derived(snapshot?.agents.find((agent) => agent.id === "reviewer") ?? null);
  const pending = $derived.by(() => {
    const rows = snapshot?.approvals.filter((approval) => approval.status === "pending") ?? [];
    return rows.reduce<Approval | null>((oldest, approval) => {
      if (!oldest || approval.created_at < oldest.created_at) {
        return approval;
      }
      return oldest;
    }, null);
  });
  const receipts = $derived.by(() => {
    const map: Record<string, Approval> = {};
    for (const approval of snapshot?.approvals ?? []) {
      if (approval.status === "pending") {
        continue;
      }
      map[`approval-requested:${approval.id}`] = approval;
    }
    return map;
  });
  const stream = $derived.by(() => {
    const events = snapshot?.events ?? [];
    const pendingKey = pending ? `approval-requested:${pending.id}` : "";
    // The daemon keeps every event. The stream renders the newest
    // STREAM_ROWS so the DOM, and each receipt card's observers, stay bounded
    // however long the demo runs.
    return [...events].reverse().slice(-STREAM_ROWS).map((event) => {
      const source = event.source.trim();
      const shortId = shortEventId(event.id);
      const millis = hlcMillis(event.hlc);
      return {
        id: event.id,
        mark: source.charAt(0).toUpperCase() || "·",
        when: millis == null ? "—" : formatStreamTime(millis),
        iso: millis == null ? "" : new Date(millis).toISOString(),
        who: source ? `${source} · ${event.kind}` : event.kind,
        detail: shortId,
        title: event.hlc,
        receipt: receipts[event.idempotency_key] ?? null,
        pendingHere: pendingKey !== "" && event.idempotency_key === pendingKey,
      };
    });
  });
  const pendingAnchored = $derived(stream.some((row) => row.pendingHere));
  const anyReceipt = $derived(stream.some((row) => row.receipt !== null));
  // Pending is waiting on a human. A decided, uncommitted approval is filing: ink-3, no dot.
  const waitingOnHuman = $derived(
    (snapshot?.approvals ?? []).some(
      (approval) => approval.agent_id === reviewer?.id && approval.status === "pending",
    ),
  );
  const filingUndo = $derived.by(() => {
    if (waitingOnHuman || reviewer == null) {
      return null;
    }
    const rows =
      snapshot?.approvals.filter((approval) => {
        return (
          approval.agent_id === reviewer.id &&
          (approval.status === "approved" || approval.status === "denied") &&
          !approval.committed
        );
      }) ?? [];
    return rows.reduce<Approval | null>((latest, approval) => {
      if (!latest || (approval.decided_at ?? 0) > (latest.decided_at ?? 0)) {
        return approval;
      }
      return latest;
    }, null);
  });
  const filingSeconds = $derived(
    filingUndo?.undo_until == null
      ? 0
      : Math.max(0, Math.ceil((filingUndo.undo_until - now) / 1000)),
  );
  const reviewerChrome = $derived.by((): ReviewerChrome | null => {
    if (reviewer == null) {
      return null;
    }
    let filing: string | null = null;
    if (filingUndo && filingSeconds > 0) {
      filing = `${filingUndo.status === "denied" ? "denied" : "approved"} · undo ${filingSeconds}s`;
    }
    return {
      waiting: waitingOnHuman,
      filing,
      working: reviewer.status === "working",
    };
  });
  const rosterAgents = $derived(
    (snapshot?.agents ?? []).map((agent) => ({
      id: agent.id,
      name: agent.name,
      status: agent.status,
    })),
  );
  const openReviews = $derived(
    (snapshot?.approvals ?? [])
      .filter((approval) => approval.status === "pending")
      .map((approval) => ({
        id: approval.id,
        agent: approval.agent_name,
        title: approval.purpose || approval.action,
      })),
  );

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

  // Inside Tauri the main window has no decision capability. A decision here
  // only brings up the card window, which signs it over IPC.
  const decidesElsewhere = tauriWindowLabel() !== null;

  async function showCardWindow(): Promise<boolean> {
    try {
      await openCardWindow();
    } catch (err) {
      error = err instanceof Error ? err.message : "The card window did not open.";
    }
    return false;
  }

  async function ondecide(id: string, decision: Decision, reason?: string): Promise<boolean> {
    if (decidesElsewhere) {
      return showCardWindow();
    }
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
    if (decidesElsewhere) {
      return showCardWindow();
    }
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
        duration: tokenMs("--dur-soft", 160),
        easing: REDUCED_FADE_EASE,
        css: (t) => `opacity: ${t};`,
      };
    }
    const easeOut = tokenEase("--ease-out");
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
    const easeOut = tokenEase("--ease-out");
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

  function syncSettings() {
    settingsOpen = globalThis.location.hash === "#settings";
  }

  onMount(() => {
    syncSettings();
    const onHash = () => syncSettings();
    globalThis.addEventListener("hashchange", onHash);
    const stopPolling = startPolling(refresh, 1000);
    const clock = setInterval(() => {
      now = Date.now();
    }, 200);
    return () => {
      globalThis.removeEventListener("hashchange", onHash);
      clearInterval(clock);
      stopPolling();
    };
  });
</script>

{#snippet approvalSlot(approval: Approval)}
  <div class={["slot", approval.status === "pending" && "over"]} {@attach flipSlot}>
    {#key approval.id}
      <ApprovalCard
        approval={approval}
        busy={deciding}
        shortcutTarget={approval.status === "pending"}
        ondecide={(decision, reason) => ondecide(approval.id, decision, reason)}
        onundo={() => onundo(approval.id)}
      />
    {/key}
  </div>
{/snippet}

<Shell
  agents={rosterAgents}
  project={reviewer?.project ?? "DasVR/NIL"}
  phase={PHASE}
  reviewer={reviewerChrome}
  pending={openReviews}
  {busy}
  stage={shell.stage}
  capture={shell.capture}
  onSimulate={shell.stage ? undefined : () => void simulate()}
>
  {#if shell.stage && snapshot == null}
    <StageThread />
  {:else}
  {#if error && !shell.stage}
    <p class="banner" role="alert">{error}</p>
  {/if}

  <main style:--flat-radius={FLAT_RADIUS}>
        <p class="section">Stream</p>
        <div class="stage" {@attach pinOverlay}>
          <ol class="stream">
            {#each stream as row (row.id)}
              {@const approval = row.receipt ?? (row.pendingHere ? pending : null)}
              {@const filed = row.receipt !== null}
              <li class={filed ? "slot-row" : "event"} animate:stepRows>
                {#if !filed}
                  <div class="row" title={row.title} in:arrive|global={{ play: primed }}>
                    <time class="when" datetime={row.iso || undefined}>{row.when}</time>
                    <span class="disc" aria-hidden="true">{row.mark}</span>
                    <div class="copy">
                      <p class="who">{row.who}</p>
                      <p class="detail">{row.detail}</p>
                    </div>
                  </div>
                {/if}
                {#if approval}
                  {@render approvalSlot(approval)}
                {/if}
              </li>
            {/each}
          </ol>
          {#if pending && !pendingAnchored}
            {@render approvalSlot(pending)}
          {:else if !pending && !anyReceipt}
            <p class="quiet-empty">
              <svg class="quiet-line" viewBox="0 0 104 10" aria-hidden="true">
                <path class="pen draw" pathLength="1" d={QUIET_LINE_PATH} />
              </svg>
              Nothing is waiting.
            </p>
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
  {/if}
  {#if settingsOpen}
    <SecretEntry />
  {/if}
</Shell>

<style>
  .banner {
    margin: 0 0 var(--s-3);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-sm);
    color: var(--ink-1);
    background: var(--paper-sunken);
  }

  main {
    padding: var(--s-4);
    padding-bottom: 96px;
    background: transparent;
  }

  .section {
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
    margin-bottom: var(--s-3);
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
    border-radius: var(--flat-radius);
    background: none;
    box-shadow: none;
    min-width: 0;
  }

  .row {
    display: grid;
    grid-template-columns: 76px 28px minmax(0, 1fr);
    column-gap: 12px;
    align-items: center;
    padding: 10px 2px 10px;
    /* 37px copy + 20px padding + the 1px rule is 58. Min-height matches the 60px receipt. */
    min-height: 60px;
    border: 0;
    border-bottom: 1px solid var(--hairline);
    border-radius: var(--flat-radius);
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
    font-weight: var(--w-regular);
    color: var(--ink-2);
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
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .detail {
    margin-top: 2px;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Filed receipt sits in the row's copy column. Same tracks as .row, without a hairline rule. */
  .slot-row {
    display: grid;
    grid-template-columns: 76px 28px minmax(0, 1fr);
    column-gap: 12px;
    padding: 0 2px;
    border: 0;
    min-height: 0;
    height: auto;
  }

  .slot-row .slot {
    grid-column: 3;
    margin-top: 0;
    min-width: 0;
    max-width: 100%;
  }

  .slot-row .slot :global(article.card) {
    max-width: 100%;
    min-width: 0;
  }

  :global(.win[data-shell-form="companion"]) main {
    padding: 0;
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
    bottom: calc(var(--composer-block, 0px) + 16px);
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

  .quiet-empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
    margin-top: var(--s-3);
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .quiet-line {
    width: 104px;
    height: 10px;
    overflow: visible;
  }

  .quiet-empty .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: var(--pen-width);
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }

  .quiet-empty .draw {
    stroke-dasharray: 1;
    stroke-dashoffset: 0;
    animation: draw-quiet var(--dur-draw) var(--ease-draw) both;
  }

  @keyframes draw-quiet {
    from {
      stroke-dashoffset: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .quiet-empty .draw {
      animation: none;
    }
  }

  .muted {
    margin-top: var(--s-2);
    color: var(--ink-2);
    font-size: var(--t-meta);
  }

  .ledger {
    font-family: var(--font-machine);
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
</style>
