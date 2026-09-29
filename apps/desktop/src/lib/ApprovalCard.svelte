<script lang="ts">
  import { tick } from "svelte";
  import { MediaQuery } from "svelte/reactivity";
  import {
    SEEN_LOCK_MS,
    actionTitle,
    effectAsk,
    effectLabel,
    effectWhy,
    formatDecisionStamp,
    formatTokens,
    formatUsd,
    holdDurationMs,
    isTextEntry,
    parseEffect,
    shortEventId,
    type Approval,
    type Decision,
  } from "./api";
  import { CHECK_PATH, CHEVRON_PATH, DENY_MARK_PATH, STRIKE_PATH, arrowFromId, type ArrowMark } from "./pen";

  interface Props {
    approval: Approval;
    busy?: boolean;
    holdMs?: number;
    shortcutTarget?: boolean;
    ondecide?: (decision: Decision, reason?: string) => Promise<boolean>;
    onundo?: () => Promise<boolean>;
  }

  let {
    approval,
    busy = false,
    holdMs = 600,
    shortcutTarget = false,
    ondecide,
    onundo,
  }: Props = $props();

  const reducedMotion = new MediaQuery("(prefers-reduced-motion: reduce)");

  const effect = $derived(parseEffect(approval.effect_class));
  const risk = $derived(effect ?? "unknown");
  const pending = $derived(approval.status === "pending");
  const title = $derived(actionTitle(approval.action));
  const monogram = $derived(approval.agent_name.trim().charAt(0).toUpperCase() || "·");
  const titleId = $derived(`approval-title-${approval.id}`);
  const reasonId = $derived(`approval-reason-${approval.id}`);

  let cardEl = $state<HTMLElement | null>(null);
  let playRise = $state(false);
  let riseNoted = false;
  let arrow = $state<ArrowMark | null>(null);
  let seenArmed = $state(false);
  let cardFocused = $state(false);
  let denyOpen = $state(false);
  let reason = $state("");
  let strike = $state(0);
  let checkOffset = $state(1);
  let committing = $state<Decision | null>(null);
  let nowMs = $state(Date.now());

  type HoldKind = "approve" | "deny";
  let holdKind = $state<HoldKind | null>(null);
  let holdFrame = 0;
  let morphTimer = 0;
  let holdSealed = false;

  const locked = $derived(busy || committing !== null);
  const duration = $derived(holdDurationMs(effect, holdMs));
  const showUndo = $derived(
    !pending &&
      !approval.committed &&
      (approval.status === "approved" || approval.status === "denied") &&
      approval.undo_until != null &&
      approval.undo_until > nowMs,
  );
  const undoSeconds = $derived(
    approval.undo_until == null ? 0 : Math.max(0, Math.ceil((approval.undo_until - nowMs) / 1000)),
  );
  const stamp = $derived(approval.decided_at == null ? "" : formatDecisionStamp(approval.decided_at));
  const decisionShort = $derived(shortEventId(approval.decision_event_id ?? ""));
  const eventLine = $derived.by(() => {
    const id = shortEventId(approval.evidence.event_id);
    return approval.evidence.kind ? `${id} · ${approval.evidence.kind}` : id;
  });
  const commandGlyph = $derived.by(() => {
    const platform = navigator.platform;
    const agent = navigator.userAgent;
    if (/Mac|iPhone|iPad/.test(platform) || /Mac OS X/.test(agent)) {
      return "⌘";
    }
    return "Ctrl";
  });
  const holdHint = $derived(`hold ${commandGlyph}↵ / hold ${commandGlyph}⌫`);
  const draftPieces = $derived.by(() => {
    const pieces: { code: boolean; text: string }[] = [];
    const pattern = /refresh\(\)/g;
    let cursor = 0;
    for (const match of approval.draft.matchAll(pattern)) {
      const index = match.index ?? 0;
      if (index > cursor) {
        pieces.push({ code: false, text: approval.draft.slice(cursor, index) });
      }
      pieces.push({ code: true, text: match[0] });
      cursor = index + match[0].length;
    }
    if (cursor < approval.draft.length) {
      pieces.push({ code: false, text: approval.draft.slice(cursor) });
    }
    if (pieces.length === 0) {
      pieces.push({ code: false, text: approval.draft });
    }
    return pieces;
  });
  const subline = $derived.by(() => {
    if (approval.status === "expired") {
      return "Reviewer will ask again on the next push.";
    }
    if (approval.status === "denied" && approval.reason) {
      return `Reason: ${approval.reason}`;
    }
    return "Decision recorded. Nothing posted (demo).";
  });
  const word = $derived.by(() => {
    switch (approval.status) {
      case "approved":
        return "Approved";
      case "denied":
        return "Denied";
      case "expired":
        return "Expired before a decision";
      case "pending":
        return "";
      default:
        return approval.status;
    }
  });

  function bindCard(node: HTMLElement): () => void {
    cardEl = node;
    if (!riseNoted) {
      riseNoted = true;
      playRise = pending;
    }
    return () => {
      window.clearTimeout(morphTimer);
      cancelAnimationFrame(holdFrame);
      if (cardEl === node) {
        cardEl = null;
      }
    };
  }

  function watchSeen(evidence: HTMLElement): () => void {
    const card = evidence.closest("article");
    if (!card) {
      return () => {};
    }
    let cardFull = false;
    let evidenceVisible = false;
    let timer = 0;
    const sync = () => {
      window.clearTimeout(timer);
      if (cardFull && evidenceVisible) {
        timer = window.setTimeout(() => {
          seenArmed = true;
        }, SEEN_LOCK_MS);
      } else {
        seenArmed = false;
      }
    };
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const full = entry.intersectionRatio >= 0.99;
          if (entry.target === card) {
            cardFull = full;
          }
          if (entry.target === evidence) {
            evidenceVisible = full;
          }
        }
        sync();
      },
      { threshold: [0, 0.99, 1] },
    );
    observer.observe(card);
    observer.observe(evidence);
    return () => {
      window.clearTimeout(timer);
      observer.disconnect();
      seenArmed = false;
    };
  }

  function watchArrow(strip: HTMLElement): () => void {
    const card = strip.closest("article");
    if (!card) {
      return () => {};
    }
    const id = approval.id;
    let current = "";
    const measure = () => {
      const next = arrowFromId(id);
      const key = `${next.width}x${next.height}:${next.shaft}:${next.head}`;
      if (key === current) {
        return;
      }
      current = key;
      arrow = next;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(card);
    return () => {
      observer.disconnect();
    };
  }

  function tickUndo(): () => void {
    nowMs = Date.now();
    const timer = window.setInterval(() => {
      nowMs = Date.now();
    }, 200);
    return () => window.clearInterval(timer);
  }

  function focusReason(node: HTMLInputElement): () => void {
    node.focus();
    return () => {};
  }

  function syncFocus(): void {
    cardFocused = document.activeElement === cardEl;
  }

  function wait(ms: number): Promise<void> {
    return new Promise((resolve) => {
      window.setTimeout(resolve, ms);
    });
  }

  function animateNumber(from: number, to: number, ms: number, apply: (value: number) => void): Promise<void> {
    if (reducedMotion.current || ms <= 0) {
      apply(to);
      return Promise.resolve();
    }
    cancelAnimationFrame(holdFrame);
    return new Promise((resolve) => {
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / ms);
        apply(from + (to - from) * t);
        if (t < 1) {
          holdFrame = requestAnimationFrame(step);
        } else {
          resolve();
        }
      };
      holdFrame = requestAnimationFrame(step);
    });
  }

  function clearMorph(): void {
    if (!cardEl) {
      return;
    }
    cardEl.style.height = "";
    cardEl.style.transition = "";
    cardEl.style.overflow = "";
  }

  function lockHeight(): number {
    if (!cardEl) {
      return 0;
    }
    const from = cardEl.offsetHeight;
    cardEl.style.overflow = "hidden";
    cardEl.style.height = `${from}px`;
    return from;
  }

  /** Intrinsic height after the content swap. A locked height makes scrollHeight lie. */
  function measureUnlocked(): number {
    if (!cardEl) {
      return 0;
    }
    const locked = cardEl.style.height;
    cardEl.style.height = "auto";
    const next = cardEl.offsetHeight;
    cardEl.style.height = locked;
    return next;
  }

  function scheduleMorph(to: number, delay: number, duration: string): void {
    window.clearTimeout(morphTimer);
    const apply = () => {
      if (!cardEl) {
        return;
      }
      if (reducedMotion.current) {
        clearMorph();
        return;
      }
      cardEl.style.transition = `height ${duration} var(--ease-out)`;
      cardEl.style.height = `${to}px`;
      morphTimer = window.setTimeout(() => clearMorph(), 700);
    };
    if (delay <= 0) {
      apply();
      return;
    }
    morphTimer = window.setTimeout(apply, delay);
  }

  async function runDecide(decision: Decision, note?: string): Promise<boolean> {
    if (!ondecide) {
      return false;
    }
    return ondecide(decision, note);
  }

  async function settleDecision(decision: Decision, note?: string): Promise<void> {
    lockHeight();
    const ok = await runDecide(decision, note);
    if (!ok) {
      committing = null;
      checkOffset = 1;
      clearMorph();
      return;
    }
    await tick();
    if (!cardEl) {
      return;
    }
    const to = measureUnlocked();
    scheduleMorph(to, reducedMotion.current ? 0 : 360, "var(--dur-stage)");
  }

  async function onApproveClick(): Promise<void> {
    if (locked || denyOpen || !pending) {
      return;
    }
    committing = "approve";
    const drawMs = reducedMotion.current ? 0 : 300;
    await animateNumber(1, 0, drawMs, (value) => {
      checkOffset = value;
    });
    if (reducedMotion.current) {
      checkOffset = 0;
      await wait(120);
    }
    await settleDecision("approve");
  }

  async function confirmDeny(): Promise<void> {
    if (locked || !pending) {
      return;
    }
    committing = "deny";
    if (strike < 1) {
      const drawMs = reducedMotion.current ? 0 : 300;
      await animateNumber(strike, 1, drawMs, (value) => {
        strike = value;
      });
      if (reducedMotion.current) {
        await wait(120);
      }
    }
    await settleDecision("deny", reason);
  }

  function openDeny(): void {
    denyOpen = true;
  }

  function back(): void {
    denyOpen = false;
    reason = "";
    strike = 0;
    cardEl?.focus();
  }

  function retract(kind: HoldKind, from: number): void {
    if (reducedMotion.current) {
      if (kind === "approve") {
        checkOffset = 1;
      } else {
        strike = 0;
      }
      return;
    }
    void animateNumber(from, 0, 160, (value) => {
      if (kind === "approve") {
        checkOffset = 1 - value;
      } else {
        strike = value;
      }
    });
  }

  function cancelHold(): void {
    if (!holdKind || holdSealed) {
      return;
    }
    const kind = holdKind;
    const from = kind === "approve" ? 1 - checkOffset : strike;
    holdKind = null;
    retract(kind, from);
  }

  function finishHold(kind: HoldKind): void {
    holdKind = null;
    holdSealed = true;
    if (kind === "approve") {
      checkOffset = 0;
      committing = "approve";
      void settleDecision("approve");
      return;
    }
    strike = 1;
    openDeny();
  }

  function startHold(kind: HoldKind): void {
    if (locked || denyOpen || !pending || !seenArmed || document.activeElement !== cardEl) {
      return;
    }
    cancelAnimationFrame(holdFrame);
    holdKind = kind;
    holdSealed = false;
    const ms = duration;
    const start = performance.now();
    const step = (now: number) => {
      if (holdKind !== kind) {
        return;
      }
      const progress = Math.min(1, (now - start) / ms);
      if (kind === "approve") {
        checkOffset = 1 - progress;
      } else {
        strike = progress;
      }
      if (progress >= 1) {
        finishHold(kind);
        return;
      }
      holdFrame = requestAnimationFrame(step);
    };
    holdFrame = requestAnimationFrame(step);
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.repeat || isTextEntry(event.target)) {
      return;
    }
    if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && (event.key === "z" || event.key === "Z")) {
      if (showUndo) {
        event.preventDefault();
        void onUndoClick();
      }
      return;
    }
    if (event.altKey && !event.metaKey && !event.ctrlKey && event.key === "ArrowDown" && shortcutTarget && pending) {
      event.preventDefault();
      cardEl?.focus();
      return;
    }
    const chord = event.metaKey || event.ctrlKey;
    if (!chord || event.altKey || !pending || denyOpen || locked) {
      return;
    }
    if (document.activeElement !== cardEl || !seenArmed) {
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      startHold("approve");
    } else if (event.key === "Backspace") {
      event.preventDefault();
      startHold("deny");
    }
  }

  function onWindowKeyup(event: KeyboardEvent): void {
    const released =
      event.key === "Enter" || event.key === "Backspace" || event.key === "Meta" || event.key === "Control";
    if (!released) {
      return;
    }
    if (holdSealed) {
      holdSealed = false;
      return;
    }
    cancelHold();
  }

  function onApproveKeydown(event: KeyboardEvent): void {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    if (!seenArmed || locked) {
      event.preventDefault();
    }
  }

  async function onUndoClick(): Promise<void> {
    if (!onundo || !showUndo) {
      return;
    }
    lockHeight();
    const ok = await onundo();
    if (!ok) {
      clearMorph();
      return;
    }
    denyOpen = false;
    reason = "";
    strike = 0;
    checkOffset = 1;
    committing = null;
    await tick();
    if (!cardEl) {
      return;
    }
    scheduleMorph(measureUnlocked(), 0, "var(--dur-soft)");
  }
</script>

<svelte:window onkeydown={onWindowKeydown} onkeyup={onWindowKeyup} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<article
  {@attach bindCard}
  class={["card", pending ? "glass" : "paper", playRise && "rise"]}
  data-risk={risk}
  tabindex={pending ? 0 : undefined}
  aria-labelledby={pending ? titleId : undefined}
  aria-label={pending ? undefined : word}
  aria-keyshortcuts={pending ? "Control+Enter Control+Backspace" : undefined}
  onfocusin={syncFocus}
  onfocusout={() => {
    queueMicrotask(() => {
      syncFocus();
      if (document.activeElement !== cardEl) {
        cancelHold();
      }
    });
  }}
>
  {#if pending}
    {#if effect === "read"}
      <p class="risk-read">Read</p>
    {:else if effect}
      <div class="risk" role="note" {@attach watchArrow}>
        <span class="dot"></span>
        <span>{effectLabel(effect)}</span>
        {#if effectWhy(effect)}
          <span class="why">· {effectWhy(effect)}</span>
        {/if}
      </div>
      {#if arrow}
        <svg
          class="arrow"
          viewBox={`0 0 ${arrow.width} ${arrow.height}`}
          aria-hidden="true"
        >
          <path class="pen trace draw" pathLength="1" d={arrow.shaft} />
          <path class="pen trace draw" pathLength="1" d={arrow.head} />
        </svg>
      {/if}
    {/if}

    <div class="inner">
      <header class="card-head">
        <div class="agent" aria-hidden="true">{monogram}</div>
        <div>
          <p class="kicker"><b>{approval.agent_name}</b> · {effect ? effectAsk(effect) : "asks before posting"}</p>
          <h2 id={titleId}>{title}</h2>
        </div>
      </header>

      <p class="purpose">{approval.purpose}</p>

      <section class="block">
        <h3>Evidence</h3>
        <dl class="evidence" {@attach watchSeen}>
          <dt>repo</dt>
          <dd>{approval.evidence.repo}</dd>
          <dt>ref</dt>
          <dd>{approval.evidence.ref}</dd>
          <dt>event</dt>
          <dd>{eventLine}</dd>
        </dl>
      </section>

      <details open>
        <summary>
          <span class="lhs">
            <svg class="chev" viewBox="0 0 12 12" aria-hidden="true">
              <path d={CHEVRON_PATH} />
            </svg>
            <span class="draft-label">
              Draft
              <svg class="strike" viewBox="0 0 130 12" preserveAspectRatio="none" aria-hidden="true">
                <path class="pen trace" pathLength="1" d={STRIKE_PATH} style:stroke-dashoffset={1 - strike} />
              </svg>
            </span>
          </span>
          {#if approval.provider === "mock"}
            <span class="prov">mock provider, not written by a model</span>
          {/if}
        </summary>
        <p class="draft">
          {#each draftPieces as part, index (index)}
            {#if part.code}
              <code>{part.text}</code>
            {:else}
              {part.text}
            {/if}
          {/each}
        </p>
      </details>

      <p class="meta">
        {approval.provider} · {approval.model} · {formatTokens(approval.input_tokens)} in / {formatTokens(approval.output_tokens)} out · {formatUsd(approval.micro_usd)}
      </p>

      {#if denyOpen}
        <div class="reason">
          <label for={reasonId}>Reason (optional)</label>
          <input
            id={reasonId}
            {@attach focusReason}
            bind:value={reason}
            class="field"
            type="text"
            autocomplete="off"
            placeholder="Optional"
            onkeydown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void confirmDeny();
              }
            }}
          />
        </div>
      {/if}

      <div class="actions">
        {#if denyOpen}
          <button class="deny" type="button" disabled={locked} onclick={back}>Back</button>
          <button class="approve" type="button" disabled={locked} onclick={() => void confirmDeny()}>
            Deny draft
          </button>
        {:else}
          <button
            class="approve"
            type="button"
            disabled={busy}
            onkeydown={onApproveKeydown}
            onclick={() => void onApproveClick()}
          >
            {#if holdKind === "approve"}
              <svg class="check" viewBox="0 0 24 24" aria-hidden="true">
                <path class="pen trace" pathLength="1" d={CHECK_PATH} style:stroke-dashoffset={checkOffset} />
              </svg>
            {/if}
            <span class="face">
              <span class={["face-idle", committing === "approve" && "gone"]} aria-hidden={committing === "approve"}>Approve draft</span>
              <span class={["face-done", committing === "approve" && "show"]} aria-hidden={committing !== "approve"}>
                <svg class="check inline" viewBox="0 0 24 24" aria-hidden="true">
                  <path class="pen trace" pathLength="1" d={CHECK_PATH} style:stroke-dashoffset="0" />
                </svg>
                Approved
              </span>
            </span>
          </button>
          <button class="deny" type="button" disabled={busy} onclick={openDeny}>Deny draft</button>
        {/if}
      </div>

      <div class="quiet">
        <p>Records your decision. Nothing is posted in this demo.</p>
        {#if cardFocused && !denyOpen}
          <p class={["hold-hint", seenArmed && "armed"]}>{holdHint}</p>
        {/if}
      </div>
    </div>
  {:else}
    <div class="receipt" {@attach tickUndo}>
      {#if approval.status === "approved"}
        <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
          <path class="pen trace draw" pathLength="1" d={CHECK_PATH} />
        </svg>
      {:else if approval.status === "denied"}
        <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
          <path class="pen trace draw" pathLength="1" d={DENY_MARK_PATH} />
        </svg>
      {:else if approval.status === "expired"}
        <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
          <path class="pen trace draw" pathLength="1" d={DENY_MARK_PATH} />
        </svg>
      {/if}
      <div class="what">
        <p class="line">
          <b class={approval.status}>{word}</b>
          {#if approval.status === "approved" || approval.status === "denied"}
            <span> · {title}</span>
          {/if}
        </p>
        <p class="sub">{subline}</p>
      </div>
      {#if stamp || decisionShort}
        <p class="stamp">{stamp}{#if stamp && decisionShort}<br />{/if}{decisionShort}</p>
      {/if}
      {#if showUndo}
        <button class="undo" type="button" onclick={() => void onUndoClick()}>
          <span class="u">Undo</span><span class="t" aria-hidden="true">{undoSeconds}s</span>
        </button>
      {/if}
    </div>
  {/if}
</article>

<style>
  .card {
    position: relative;
    color: var(--ink-1);
    transition:
      background-color var(--dur-soft) var(--ease-in-out),
      border-radius var(--dur-soft) var(--ease-in-out),
      box-shadow var(--dur-soft) var(--ease-in-out),
      padding var(--dur-soft) var(--ease-in-out),
      backdrop-filter var(--dur-soft) var(--ease-in-out);
  }

  .card.glass {
    background: var(--glass-fill);
    -webkit-backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    border-radius: var(--r-xl);
    box-shadow: var(--glass-edge), var(--shadow-float);
    padding: var(--s-2) var(--s-2) var(--s-5);
  }

  .card.paper {
    background: var(--convex), var(--paper-raised);
    backdrop-filter: none;
    border-radius: var(--r-md);
    border: 1px solid var(--hairline);
    box-shadow: var(--highlight-top), var(--shadow-puff);
    min-height: 52px;
    padding: 10px 14px 10px 12px;
  }

  .card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .card.rise {
    animation: rise var(--dur-stage) var(--ease-out) both;
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(12px) scale(0.98);
      -webkit-backdrop-filter: blur(0px) saturate(100%);
      backdrop-filter: blur(0px) saturate(100%);
    }
  }

  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    .card.glass {
      background: var(--glass-fill-solid);
    }
  }

  .inner {
    padding: 0 var(--s-4);
  }

  .risk {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding: 8px 14px;
    border-radius: var(--r-lg);
    background: var(--risk-bg);
    color: var(--risk-external);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  .card[data-risk="write_local"] .risk {
    color: var(--risk-write-local);
  }

  .card[data-risk="destructive"] .risk {
    color: var(--risk-destructive);
    background: var(--risk-destructive-bg);
  }

  .risk-read {
    margin: 0;
    padding: 8px 14px;
    color: var(--risk-read);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }

  .why {
    color: var(--ink-2);
    font-weight: var(--w-medium);
  }

  .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: var(--pen-width);
    stroke-linecap: round;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }

  .trace {
    stroke-dasharray: 1;
  }

  .draw {
    stroke-dashoffset: 0;
    animation: draw var(--dur-draw) var(--ease-draw) both;
  }

  .arrow .draw {
    animation-delay: calc(var(--dur-stage) + 120ms);
  }

  @keyframes draw {
    from {
      stroke-dashoffset: 1;
    }
  }

  .arrow {
    position: absolute;
    right: 34px;
    top: 38px;
    width: 58px;
    height: 46px;
    overflow: visible;
    pointer-events: none;
  }

  .card-head {
    display: flex;
    gap: var(--s-3);
    align-items: flex-start;
    margin-top: var(--s-4);
    padding-right: 70px;
  }

  .agent {
    width: 32px;
    height: 32px;
    flex: none;
    border-radius: var(--r-pill);
    background: var(--convex), var(--paper-sunken);
    box-shadow: var(--highlight-top), 0 0 0 1px var(--hairline);
    display: grid;
    place-items: center;
    font-size: 13px;
    line-height: 1;
    font-weight: var(--w-semibold);
    color: var(--ink-2);
  }

  .kicker {
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-medium);
  }

  .kicker b {
    color: var(--ink-1);
    font-weight: var(--w-semibold);
  }

  h2 {
    font-size: var(--t-lead);
    line-height: var(--lh-lead);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
    margin-top: 1px;
  }

  .purpose {
    margin-top: var(--s-3);
    color: var(--ink-2);
  }

  .block {
    margin-top: var(--s-4);
  }

  h3 {
    margin-bottom: var(--s-2);
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  .evidence {
    font-family: var(--font-machine);
    font-size: 12px;
    line-height: 1.6;
    color: var(--ink-1);
    background: rgb(237 231 221 / 0.94);
    border-radius: var(--r-sm);
    padding: 10px 12px;
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 14px;
  }

  .evidence dt {
    color: var(--ink-2);
  }

  details {
    margin-top: var(--s-4);
  }

  summary {
    list-style: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: var(--s-2);
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  summary::-webkit-details-marker {
    display: none;
  }

  .lhs {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .chev {
    width: 12px;
    height: 12px;
    transition: transform var(--dur-base) var(--ease-out);
  }

  .chev path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  details[open] .chev {
    transform: rotate(90deg);
  }

  .draft-label {
    position: relative;
  }

  .strike {
    position: absolute;
    left: -2px;
    top: 50%;
    width: calc(100% + 10px);
    height: 12px;
    transform: translateY(-50%);
    overflow: visible;
    pointer-events: none;
  }

  .prov {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    font-weight: var(--w-regular);
    color: var(--ink-2);
    background: var(--paper-sunken);
    padding: 3px 7px;
    border-radius: var(--r-xs);
  }

  .draft {
    white-space: pre-wrap;
    padding: 12px 14px;
    border-radius: var(--r-sm);
    background: rgb(251 249 245 / 0.94);
    box-shadow: 0 0 0 1px var(--hairline);
  }

  .draft code {
    font-family: var(--font-machine);
    font-size: 12.5px;
  }

  .meta {
    margin-top: var(--s-3);
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-2);
  }

  .actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--s-2);
    margin-top: var(--s-5);
  }

  button {
    height: 40px;
    padding: 0 var(--s-4);
    border-radius: var(--r-md);
    font-size: var(--t-body);
    line-height: 1;
    font-weight: var(--w-semibold);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    position: relative;
    transition:
      transform var(--dur-base) var(--ease-out),
      box-shadow var(--dur-base) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out);
  }

  .approve {
    background:
      linear-gradient(180deg, rgb(255 255 255 / 0.1), rgb(255 255 255 / 0)),
      var(--ink-1);
    color: var(--paper-raised);
    border: 1.5px solid var(--ink-1);
    box-shadow:
      inset 0 1px 0 rgb(255 255 255 / 0.16),
      0 1px 2px rgb(var(--shade) / 0.18),
      0 6px 14px -6px rgb(var(--shade) / 0.35);
  }

  .deny {
    background: var(--convex), var(--paper-raised);
    color: var(--ink-1);
    border: 1.5px solid var(--ink-1);
    box-shadow:
      var(--highlight-top),
      0 1px 2px rgb(var(--shade) / 0.1),
      0 6px 14px -6px rgb(var(--shade) / 0.18);
  }

  button:hover:not(:disabled) {
    transform: translateY(-1px);
    transition-duration: var(--dur-fast);
  }

  .approve:hover:not(:disabled) {
    box-shadow:
      inset 0 1px 0 rgb(255 255 255 / 0.16),
      0 2px 4px rgb(var(--shade) / 0.18),
      0 10px 20px -8px rgb(var(--shade) / 0.4);
  }

  .deny:hover:not(:disabled) {
    border-color: var(--ink-1);
    box-shadow:
      var(--highlight-top),
      0 2px 3px rgb(var(--shade) / 0.16),
      0 8px 16px -8px rgb(var(--shade) / 0.28);
  }

  .approve:active:not(:disabled),
  .deny:active:not(:disabled) {
    box-shadow: var(--shadow-press);
  }

  button:active:not(:disabled) {
    transform: scale(0.97, 0.955);
    box-shadow: var(--shadow-press);
    transition-duration: var(--dur-fast);
    transition-timing-function: var(--ease-press);
  }

  button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .approve .pen {
    stroke: var(--paper-raised);
  }

  .check {
    width: 18px;
    height: 18px;
    position: absolute;
    left: 18px;
  }

  .check.inline {
    position: static;
  }

  .face {
    display: grid;
    place-items: center;
  }

  .face-idle,
  .face-done {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    transition: opacity var(--dur-soft) var(--ease-in-out);
  }

  .face-done {
    opacity: 0;
  }

  .face-idle.gone {
    opacity: 0;
  }

  .face-done.show {
    opacity: 1;
  }

  .quiet {
    margin-top: var(--s-3);
    display: flex;
    justify-content: center;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 10px;
    text-align: center;
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .hold-hint {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .hold-hint.armed {
    color: var(--ink-2);
  }

  .reason {
    margin-top: var(--s-4);
    animation: reveal var(--dur-base) var(--ease-out);
  }

  @keyframes reveal {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }

  .reason label {
    display: block;
    margin-bottom: 6px;
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  .field {
    width: 100%;
    height: 40px;
    border-radius: var(--r-md);
    border: 0;
    background: var(--paper-raised);
    box-shadow:
      inset 0 1px 2px rgb(var(--shade) / 0.08),
      0 0 0 1px var(--hairline-strong);
    padding: 0 12px;
    color: var(--ink-1);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .field::placeholder {
    color: var(--ink-2);
  }

  .receipt {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 0;
  }

  .mark {
    width: 24px;
    height: 24px;
    flex: none;
  }

  .what {
    flex: 1;
    min-width: 0;
  }

  .line {
    font-weight: var(--w-regular);
  }

  .what b {
    font-weight: var(--w-semibold);
  }

  .what b.approved {
    color: var(--success);
  }

  .what b.denied {
    color: var(--ink-1);
  }

  .what b.expired {
    color: var(--warning);
  }

  .sub {
    color: var(--ink-3);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .stamp {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    text-align: right;
    white-space: nowrap;
  }

  .undo {
    display: inline-block;
    gap: 0;
    height: auto;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    color: var(--accent);
    font-size: var(--t-meta);
    line-height: 1;
    font-weight: var(--w-semibold);
    text-decoration: none;
    white-space: nowrap;
  }

  .undo .u {
    text-decoration: underline;
    text-underline-offset: 3px;
    text-decoration-thickness: 1px;
  }

  .undo:hover:not(:disabled),
  .undo:active:not(:disabled) {
    transform: none;
    box-shadow: none;
  }

  .undo .t {
    font-family: var(--font-machine);
    font-weight: var(--w-regular);
    color: var(--ink-3);
    text-decoration: none;
    display: inline-block;
    margin-left: 4px;
  }

  @media (prefers-reduced-motion: reduce) {
    .card.rise {
      animation: fade var(--dur-stage) linear both;
    }

    @keyframes fade {
      from {
        opacity: 0;
      }
    }

    .draw,
    .arrow .draw {
      animation: pen-fade 120ms linear both;
      stroke-dashoffset: 0;
    }

    @keyframes pen-fade {
      from {
        opacity: 0;
      }
    }

    .reason,
    .chev {
      animation: none;
      transition: none;
    }

    button:hover:not(:disabled),
    button:active:not(:disabled) {
      transform: none;
    }

    .face-idle,
    .face-done {
      transition: none;
    }
  }
</style>
