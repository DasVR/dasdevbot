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
  import { tokenEase, tokenMs } from "./cssTokens";
  import {
    CHECK_PATH,
    CHEVRON_PATH,
    DELETE_KEY_PATH,
    DENY_MARK_PATH,
    ENTER_KEY_PATH,
    STRIKE_PATH,
    arrowFromId,
    type ArrowMark,
  } from "./pen";

  /** Risk arrow waits after the card rise. No default-motion token is 120ms. */
  const ARROW_DELAY = "120ms";
  /** Arrow head draws after the shaft. No default token is 160ms. */
  const ARROW_HEAD_DRAW = "160ms";
  /** Reduced-motion fades. tokens.css has no linear easing token. */
  const REDUCED_FADE_EASE = "linear";
  /** Evidence is off the 11 / 12.5 / 14 type ramp. */
  const EVIDENCE_SIZE = "12px";
  /** Monogram is off the 11 / 12.5 / 14 type ramp. */
  const MONOGRAM_SIZE = "13px";
  /** Line icons. --pen-width is 1.75 and is only the pen. */
  const ICON_STROKE = "1.5";
  /** Pen marks. Same weight as --pen-width. Not a new token. */
  const PEN_STROKE = "1.75px";
  /** Action area and evidence must be fully in the viewport before the dwell can start. */
  const ACTION_SEEN_RATIO = 0.99;

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
    holdMs,
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
  let actionsEl: HTMLElement | null = null;
  let actionsFull = false;
  let evidenceVisible = false;
  let seenTimer = 0;
  let dwelling = false;
  let sightCut = false;

  const locked = $derived(busy || committing !== null);
  const duration = $derived(holdDurationMs(effect, holdMs));
  const showUndo = $derived(
    !pending &&
      !approval.committed &&
      (approval.status === "approved" || approval.status === "denied") &&
      approval.undo_until != null &&
      approval.undo_until > nowMs,
  );
  const floating = $derived(pending || showUndo);
  const undoSeconds = $derived(
    approval.undo_until == null ? 0 : Math.max(0, Math.ceil((approval.undo_until - nowMs) / 1000)),
  );
  const stamp = $derived(approval.decided_at == null ? "" : formatDecisionStamp(approval.decided_at));
  const decisionShort = $derived(shortEventId(approval.decision_event_id ?? ""));
  const eventLine = $derived.by(() => {
    const id = shortEventId(approval.evidence.event_id);
    return approval.evidence.kind ? `${id} · ${approval.evidence.kind}` : id;
  });
  const macModifier = $derived.by(() => {
    const platform = navigator.platform;
    const agent = navigator.userAgent;
    return /Mac|iPhone|iPad/.test(platform) || /Mac OS X/.test(agent);
  });
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

  function ancestorBlocksSight(card: HTMLElement): boolean {
    if (card.closest("[inert]")) {
      return true;
    }
    let opacity = 1;
    let node: HTMLElement | null = card;
    while (node) {
      const style = getComputedStyle(node);
      if (style.visibility === "hidden" || style.display === "none") {
        return true;
      }
      const value = Number.parseFloat(style.opacity);
      if (Number.isFinite(value)) {
        opacity *= value;
      }
      node = node.parentElement;
    }
    return opacity <= 0;
  }

  function actionCovered(actions: HTMLElement, card: HTMLElement): boolean {
    const box = actions.getBoundingClientRect();
    if (box.width <= 0 || box.height <= 0) {
      return true;
    }
    if (box.bottom <= 0 || box.right <= 0 || box.top >= window.innerHeight || box.left >= window.innerWidth) {
      return true;
    }
    const x = box.left + box.width / 2;
    const y = box.top + box.height / 2;
    const hit = document.elementFromPoint(x, y);
    return !(hit instanceof Node && card.contains(hit));
  }

  function undoSight(card: HTMLElement): boolean {
    if (document.visibilityState !== "visible" || sightCut) {
      return false;
    }
    const win = card.closest(".win");
    if (win instanceof HTMLElement) {
      const form = win.dataset.shellForm;
      if (form === "pill" || form === "companion") {
        return false;
      }
    }
    if (ancestorBlocksSight(card)) {
      return false;
    }
    const box = card.getBoundingClientRect();
    if (box.width <= 0 || box.height <= 0) {
      return false;
    }
    if (box.bottom <= 0 || box.right <= 0 || box.top >= window.innerHeight || box.left >= window.innerWidth) {
      return false;
    }
    const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
    return hit instanceof Node && card.contains(hit);
  }

  function cardCanBeSeen(card: HTMLElement): boolean {
    if (document.visibilityState !== "visible" || sightCut) {
      return false;
    }
    if (!actionsFull || !evidenceVisible) {
      return false;
    }
    const win = card.closest(".win");
    if (win instanceof HTMLElement) {
      const form = win.dataset.shellForm;
      if (form === "pill" || form === "companion") {
        return false;
      }
    }
    if (ancestorBlocksSight(card)) {
      return false;
    }
    if (actionsEl != null && actionCovered(actionsEl, card)) {
      return false;
    }
    return true;
  }

  function loseSight(): void {
    window.clearTimeout(seenTimer);
    seenTimer = 0;
    dwelling = false;
    seenArmed = false;
    cancelAnimationFrame(holdFrame);
    if (!holdSealed) {
      holdKind = null;
      checkOffset = 1;
      strike = 0;
    }
    const active = document.activeElement;
    if (cardEl != null && active instanceof HTMLElement && cardEl.contains(active)) {
      active.blur();
    }
  }

  function syncSight(): void {
    if (!pending || cardEl == null) {
      return;
    }
    if (!cardCanBeSeen(cardEl)) {
      loseSight();
      return;
    }
    if (seenArmed || dwelling) {
      return;
    }
    dwelling = true;
    seenTimer = window.setTimeout(() => {
      dwelling = false;
      if (cardEl != null && cardCanBeSeen(cardEl)) {
        seenArmed = true;
      } else {
        seenArmed = false;
      }
    }, SEEN_LOCK_MS);
  }

  function watchSeen(evidence: HTMLElement): () => void {
    const card = evidence.closest("article");
    const actions = card?.querySelector(".actions");
    if (!(card instanceof HTMLElement) || !(actions instanceof HTMLElement)) {
      return () => {};
    }
    actionsEl = actions;
    const sync = () => {
      syncSight();
    };
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const full = entry.intersectionRatio >= ACTION_SEEN_RATIO;
          if (entry.target === actions) {
            actionsFull = full;
          }
          if (entry.target === evidence) {
            evidenceVisible = full;
          }
        }
        sync();
      },
      { threshold: [0, ACTION_SEEN_RATIO, 1] },
    );
    observer.observe(actions);
    observer.observe(evidence);
    const hideWatch = new MutationObserver(sync);
    const watched = ["style", "inert", "data-shell-form"];
    hideWatch.observe(card, { attributes: true, attributeFilter: ["style", "inert"] });
    let parent = card.parentElement;
    while (parent) {
      hideWatch.observe(parent, { attributes: true, attributeFilter: watched });
      parent = parent.parentElement;
    }
    return () => {
      window.clearTimeout(seenTimer);
      seenTimer = 0;
      dwelling = false;
      observer.disconnect();
      hideWatch.disconnect();
      if (actionsEl === actions) {
        actionsEl = null;
      }
      actionsFull = false;
      evidenceVisible = false;
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
    syncSight();
  }

  function onWindowBlur(): void {
    sightCut = true;
    syncSight();
  }

  function onWindowFocus(): void {
    sightCut = false;
    syncSight();
  }

  function onVisibilityChange(): void {
    syncSight();
  }

  function wait(ms: number): Promise<void> {
    return new Promise((resolve) => {
      window.setTimeout(resolve, ms);
    });
  }

  function animateNumber(
    from: number,
    to: number,
    ms: number,
    apply: (value: number) => void,
    ease: (t: number) => number = (t) => t,
  ): Promise<void> {
    if (reducedMotion.current || ms <= 0) {
      apply(to);
      return Promise.resolve();
    }
    cancelAnimationFrame(holdFrame);
    return new Promise((resolve) => {
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / ms);
        const curved = ease(t);
        apply(from + (to - from) * curved);
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
    cardEl.style.opacity = "";
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

  /**
   * Final paper/glass height. Measuring the live card catches padding mid-transition
   * (68px) and the later clear snaps to 60.
   */
  function measureSettled(): number {
    if (!cardEl) {
      return 0;
    }
    const live = cardEl;
    const clone = live.cloneNode(true);
    if (!(clone instanceof HTMLElement) || !live.parentElement) {
      return live.offsetHeight;
    }
    clone.style.transition = "none";
    clone.style.animation = "none";
    clone.style.height = "auto";
    clone.style.width = `${live.offsetWidth}px`;
    clone.style.position = "absolute";
    clone.style.left = "0";
    clone.style.top = "0";
    clone.style.visibility = "hidden";
    clone.style.pointerEvents = "none";
    live.parentElement.appendChild(clone);
    const next = clone.offsetHeight;
    clone.remove();
    return next;
  }

  function finishMorph(to: number, tries = 0): void {
    if (!cardEl) {
      return;
    }
    const node = cardEl;
    const locked = node.style.height;
    node.style.transition = "none";
    node.style.height = "auto";
    const settled = node.offsetHeight;
    if (Math.abs(settled - to) <= 1 || tries >= 8) {
      node.style.height = "";
      node.style.overflow = "";
      node.style.opacity = "";
      void node.offsetHeight;
      node.style.transition = "";
      return;
    }
    node.style.height = locked || `${to}px`;
    void node.offsetHeight;
    node.style.transition = "";
    morphTimer = window.setTimeout(() => finishMorph(to, tries + 1), 50);
  }

  function scheduleMorph(to: number): void {
    window.clearTimeout(morphTimer);
    if (!cardEl || to <= 0) {
      clearMorph();
      return;
    }
    const node = cardEl;
    if (reducedMotion.current) {
      // Rise fill owns opacity. Clear it so the receipt can fade in.
      node.style.animation = "none";
      node.style.transition = "none";
      node.style.height = `${to}px`;
      node.style.overflow = "";
      node.style.opacity = "0";
      void node.offsetHeight;
      node.style.transition = `opacity var(--dur-soft) ${REDUCED_FADE_EASE}`;
      node.style.opacity = "1";
      morphTimer = window.setTimeout(() => finishMorph(to), 180);
      return;
    }
    node.style.overflow = "hidden";
    node.style.transition =
      "height var(--dur-soft) var(--ease-out), background-color var(--dur-soft) var(--ease-in-out), box-shadow var(--dur-soft) var(--ease-in-out), border-radius var(--dur-soft) var(--ease-in-out), backdrop-filter var(--dur-soft) var(--ease-in-out)";
    node.style.height = `${to}px`;
    const done = (event: TransitionEvent) => {
      if (event.target !== node || event.propertyName !== "height") {
        return;
      }
      node.removeEventListener("transitionend", done);
      window.clearTimeout(morphTimer);
      finishMorph(to);
    };
    node.addEventListener("transitionend", done);
    morphTimer = window.setTimeout(() => {
      node.removeEventListener("transitionend", done);
      finishMorph(to);
    }, 480);
  }

  async function runDecide(decision: Decision, note?: string): Promise<boolean> {
    if (!ondecide) {
      return false;
    }
    return ondecide(decision, note);
  }

  function decisionAllowed(): boolean {
    return seenArmed && cardEl != null && document.visibilityState === "visible" && cardCanBeSeen(cardEl);
  }

  async function settleDecision(decision: Decision, note?: string): Promise<void> {
    if (!decisionAllowed()) {
      loseSight();
      committing = null;
      checkOffset = 1;
      strike = 0;
      return;
    }
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
    scheduleMorph(measureSettled());
  }

  async function onApproveClick(): Promise<void> {
    if (locked || denyOpen || !pending || !decisionAllowed()) {
      return;
    }
    committing = "approve";
    const drawMs = reducedMotion.current ? 0 : tokenMs("--dur-draw", 300);
    await animateNumber(
      1,
      0,
      drawMs,
      (value) => {
        checkOffset = value;
      },
      tokenEase("--ease-draw"),
    );
    if (reducedMotion.current) {
      checkOffset = 0;
      await wait(tokenMs("--dur-base", 120));
    }
    await settleDecision("approve");
  }

  async function confirmDeny(): Promise<void> {
    if (locked || !pending || !decisionAllowed()) {
      return;
    }
    committing = "deny";
    if (strike < 1) {
      const drawMs = reducedMotion.current ? 0 : tokenMs("--dur-draw", 300);
      await animateNumber(
        strike,
        1,
        drawMs,
        (value) => {
          strike = value;
        },
        tokenEase("--ease-draw"),
      );
      if (reducedMotion.current) {
        await wait(tokenMs("--dur-base", 120));
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
    void animateNumber(
      from,
      0,
      tokenMs("--dur-retract", 160),
      (value) => {
        if (kind === "approve") {
          checkOffset = 1 - value;
        } else {
          strike = value;
        }
      },
      tokenEase("--ease-exit"),
    );
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
    if (document.activeElement !== cardEl || !decisionAllowed()) {
      loseSight();
      return;
    }
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
    if (locked || denyOpen || !pending || document.activeElement !== cardEl || !decisionAllowed()) {
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
      if (document.activeElement !== cardEl || !decisionAllowed()) {
        loseSight();
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
      if (showUndo && cardEl != null && document.activeElement === cardEl && undoSight(cardEl)) {
        event.preventDefault();
        void onUndoClick();
      }
      return;
    }
    if (event.altKey && !event.metaKey && !event.ctrlKey && event.key === "ArrowDown" && shortcutTarget && pending) {
      event.preventDefault();
      if (cardEl == null || !cardCanBeSeen(cardEl)) {
        return;
      }
      cardEl.focus();
    }
  }

  function onCardKeydown(event: KeyboardEvent): void {
    if (event.repeat || isTextEntry(event.target) || document.activeElement !== cardEl) {
      return;
    }
    const chord = (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey;
    if (!chord || !pending || denyOpen || locked) {
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
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
    }
  }

  async function onUndoClick(): Promise<void> {
    if (!onundo || !showUndo || cardEl == null || !undoSight(cardEl)) {
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
    scheduleMorph(measureSettled());
  }
</script>

<svelte:window
  onkeydown={onWindowKeydown}
  onkeyup={onWindowKeyup}
  onblur={onWindowBlur}
  onfocus={onWindowFocus}
/>
<svelte:document onvisibilitychange={onVisibilityChange} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<article
  {@attach bindCard}
  class={["card", floating ? "glass" : "paper", !pending && "receipt", playRise && "rise"]}
  style:--arrow-delay={ARROW_DELAY}
  style:--arrow-head-draw={ARROW_HEAD_DRAW}
  style:--reduced-fade={REDUCED_FADE_EASE}
  style:--evidence-size={EVIDENCE_SIZE}
  style:--monogram-size={MONOGRAM_SIZE}
  style:--icon-stroke={ICON_STROKE}
  style:--pen-stroke={PEN_STROKE}
  data-risk={risk}
  tabindex={pending || showUndo ? 0 : undefined}
  aria-labelledby={pending ? titleId : undefined}
  aria-label={pending ? undefined : word}
  aria-keyshortcuts={pending ? "Control+Enter Control+Backspace" : undefined}
  onkeydown={onCardKeydown}
  onfocusin={syncFocus}
  onfocusout={() => {
    queueMicrotask(() => {
      syncFocus();
      if (document.activeElement !== cardEl) {
        if (cardEl == null || !cardCanBeSeen(cardEl)) {
          loseSight();
        } else {
          cancelHold();
        }
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
          <path class="pen trace shaft" pathLength="1" d={arrow.shaft} />
          <path class="pen trace head" pathLength="1" d={arrow.head} />
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
            <span class="prov">mock · demo draft, not written by a model</span>
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
            {#if holdKind === "approve" || checkOffset < 1}
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
          {#snippet modifier()}
            <kbd>{macModifier ? "⌘" : "Ctrl"}</kbd>
          {/snippet}
          {#snippet enterKey()}
            <svg class="key" viewBox="0 0 16 16" role="img" aria-label="Enter">
              <path d={ENTER_KEY_PATH} />
            </svg>
          {/snippet}
          {#snippet deleteKey()}
            <svg class="key" viewBox="0 0 16 16" role="img" aria-label="Delete">
              <path d={DELETE_KEY_PATH} />
            </svg>
          {/snippet}
          <p class={["hold-hint", seenArmed && "armed"]}>
            {#if seenArmed}
              hold {@render modifier()} {@render enterKey()} approve · hold {@render modifier()} {@render deleteKey()} deny
            {:else}
              hold {@render modifier()} {@render enterKey()} unlocks once the evidence has been on screen
            {/if}
          </p>
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

  .card.glass.receipt {
    padding: 10px 14px 10px 12px;
    border: 1px solid var(--hairline);
    min-height: 52px;
  }

  .card.paper {
    background: var(--convex), var(--paper-raised);
    -webkit-backdrop-filter: blur(0px) saturate(100%);
    backdrop-filter: blur(0px) saturate(100%);
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
    animation: rise var(--dur-stage) var(--ease-out) backwards;
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
    border-radius: var(--r-pill);
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
    stroke-width: var(--pen-stroke);
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

  .arrow .shaft {
    stroke-dashoffset: 0;
    animation: draw var(--dur-draw) var(--ease-draw) both;
    animation-delay: calc(var(--dur-stage) + var(--arrow-delay));
  }

  .arrow .head {
    stroke-dashoffset: 0;
    animation: draw var(--arrow-head-draw) var(--ease-draw) both;
    animation-delay: calc(var(--dur-stage) + var(--arrow-delay) + var(--dur-draw));
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
    font-size: var(--monogram-size);
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
    font-size: var(--evidence-size);
    line-height: 1.6;
    color: var(--ink-1);
    background: color-mix(in oklab, var(--paper-sunken) 94%, transparent);
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

  .chev path,
  .key path {
    fill: none;
    stroke: currentColor;
    stroke-width: var(--icon-stroke);
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .key path {
    vector-effect: non-scaling-stroke;
  }

  .key {
    width: 12px;
    height: 12px;
    display: inline-block;
    vertical-align: -2px;
    overflow: visible;
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
    background: color-mix(in oklab, var(--paper-raised) 94%, transparent);
    box-shadow: 0 0 0 1px var(--hairline);
  }

  .draft code {
    font-family: var(--font-machine);
    font-size: var(--t-meta);
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
    background: var(--ink-1);
    color: var(--paper-raised);
    border: 1.5px solid var(--ink-1);
    box-shadow: var(--highlight-top), var(--shadow-puff);
  }

  .deny {
    background: var(--convex), var(--paper-raised);
    color: var(--ink-1);
    border: 1.5px solid var(--ink-1);
    box-shadow: var(--highlight-top), var(--shadow-puff);
  }

  button:hover:not(:disabled) {
    transform: translateY(-1px);
    transition-duration: var(--dur-fast);
  }

  .approve:hover:not(:disabled) {
    box-shadow: var(--highlight-top), var(--shadow-float);
  }

  .deny:hover:not(:disabled) {
    border-color: var(--ink-1);
    box-shadow: var(--highlight-top), var(--shadow-puff);
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
    animation: hint-in var(--dur-base) var(--ease-out) both;
  }

  .hold-hint.armed {
    color: var(--ink-2);
  }

  .hold-hint kbd {
    font: inherit;
    color: inherit;
  }

  @keyframes hint-in {
    from {
      opacity: 0;
    }
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
    box-shadow: var(--shadow-press), 0 0 0 1px var(--hairline-strong);
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
    min-width: 0;
    max-width: 100%;
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
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
      animation: fade var(--dur-stage) var(--reduced-fade) both;
    }

    @keyframes fade {
      from {
        opacity: 0;
      }
    }

    .draw,
    .arrow .shaft,
    .arrow .head {
      animation: pen-fade var(--dur-base) var(--reduced-fade) both;
      animation-delay: 0s;
      stroke-dashoffset: 0;
    }

    @keyframes pen-fade {
      from {
        opacity: 0;
      }
    }

    .reason,
    .chev,
    .hold-hint {
      animation: none;
      transition: none;
    }

    button:hover:not(:disabled),
    button:active:not(:disabled),
    details[open] .chev {
      transform: none;
    }

    .approve:active:not(:disabled),
    .deny:active:not(:disabled) {
      transition: none;
    }

    .deny:active:not(:disabled) {
      background: color-mix(in oklab, var(--ink-1) 14%, var(--paper-raised));
    }

    .face-idle,
    .face-done {
      transition: none;
    }
  }
</style>
