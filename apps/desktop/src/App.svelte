<script lang="ts">
  import { onMount, tick } from "svelte";
  import { flip, type AnimationConfig } from "svelte/animate";
  import { linear } from "svelte/easing";
  import type { TransitionConfig } from "svelte/transition";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import { tokenEase, tokenMs } from "./lib/cssTokens";
  import { devWindowsHello } from "./lib/hello";
  import { QUIET_LINE_PATH } from "./lib/pen";
  import ReviewSheet from "./lib/ReviewSheet.svelte";
  import {
    actionTitle,
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
  import {
    byOldest,
    destructiveApprovals,
    formatExpires,
    isDestructive,
    isExpiringSoon,
    queueSummary,
    remainingMs,
    toSheetRow,
    waitingApprovals,
  } from "./lib/queue";

  /** Flat rows and the roster. tokens.css has no zero radius. */
  const FLAT_RADIUS = "0";
  /** Reduced-motion fades. tokens.css has no linear easing token. */
  const REDUCED_FADE_EASE = linear;

  let snapshot = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let deciding = $state(false);
  let primed = $state(false);
  let now = $state(Date.now());
  let daemonLost = $state(false);
  let lostAt = $state<number | null>(null);
  let sheetOpen = $state(false);
  let sheetVisible = $state(false);
  let landing = $state(false);
  let focusedId = $state<string | null>(null);
  let threadAgentId = $state<string | null>(null);
  let switchAt = $state(0);
  let focusAt = $state(0);
  let landedAt = $state<number | null>(null);
  let toastText = $state<string | null>(null);
  let toastReview = $state(false);
  let expiredPinned = $state<string[]>([]);
  let carryDone = $state<string[]>([]);
  let doneIds = $state<string[]>([]);

  const expiresFrozen = new Map<string, string>();
  const expiringToasted = new Set<string>();
  const localDecisions = new Set<string>();
  let previousStatus = new Map<string, string>();
  let decisionsSeeded = false;
  let resolveOutro: (() => void) | null = null;

  const reviewer = $derived(snapshot?.agents.find((agent) => agent.id === "reviewer") ?? null);
  const waiting = $derived(waitingApprovals(snapshot?.approvals ?? [], now));
  const blocked = $derived(destructiveApprovals(snapshot?.approvals ?? []));
  const seenEnabled = $derived(!sheetVisible && !landing && !daemonLost);
  // A chosen card that leaves the queue must not be replaced. That would auto-advance.
  const pending = $derived.by(() => {
    if (focusedId) {
      return waiting.find((approval) => approval.id === focusedId) ?? null;
    }
    return waiting[0] ?? null;
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
    return [...events].reverse().map((event) => {
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
        key: event.idempotency_key,
        receipt: receipts[event.idempotency_key] ?? null,
        pendingHere: pendingKey !== "" && event.idempotency_key === pendingKey,
      };
    });
  });
  const pendingAnchored = $derived(stream.some((row) => row.pendingHere));
  const anyReceipt = $derived(stream.some((row) => row.receipt !== null));
  const undoable = $derived.by(() => {
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
  const approvalByKey = $derived.by(() => {
    const map: Record<string, Approval> = {};
    for (const approval of snapshot?.approvals ?? []) {
      map[`approval-requested:${approval.id}`] = approval;
    }
    return map;
  });
  const latestDecidedId = $derived.by(() => {
    const rows = (snapshot?.approvals ?? []).filter(
      (approval) => approval.status === "approved" || approval.status === "denied",
    );
    const latest = rows.reduce<Approval | null>((best, approval) => {
      if (!best || (approval.decided_at ?? 0) > (best.decided_at ?? 0)) {
        return approval;
      }
      return best;
    }, null);
    return latest?.id ?? null;
  });
  const sheetRows = $derived.by(() => {
    const approvals = snapshot?.approvals ?? [];
    const pins = approvals.filter((approval) => expiredPinned.includes(approval.id));
    const merged = [...waiting];
    for (const pin of pins) {
      if (!merged.some((row) => row.id === pin.id)) {
        merged.push(pin);
      }
    }
    merged.sort(byOldest);
    return merged.map((approval) => {
      let frozen = expiresFrozen.get(approval.id) ?? null;
      if (!frozen && isExpiringSoon(approval, now)) {
        const remaining = remainingMs(approval, now);
        if (remaining != null) {
          frozen = formatExpires(remaining);
          expiresFrozen.set(approval.id, frozen);
        }
      }
      return toSheetRow(approval, frozen, expiredPinned.includes(approval.id));
    });
  });
  const doneRows = $derived.by(() => {
    const approvals = snapshot?.approvals ?? [];
    return doneIds
      .map((id) => approvals.find((approval) => approval.id === id))
      .filter((approval): approval is Approval => approval != null)
      .map((approval) => toSheetRow(approval, null, true));
  });
  const daemonLine = $derived(
    daemonLost && lostAt != null
      ? `Showing what was waiting at ${formatStreamTime(lostAt)}. Your decisions are saved.`
      : null,
  );

  async function refresh(): Promise<void> {
    try {
      snapshot = await getSnapshot();
      error = null;
      daemonLost = false;
      lostAt = null;
      noteRemoteDecisions(snapshot.approvals);
      if (focusedId == null) {
        focusedId = waitingApprovals(snapshot.approvals, Date.now())[0]?.id ?? null;
      }
      if (threadAgentId == null) {
        threadAgentId = snapshot.agents.find((agent) => agent.id === "reviewer")?.id ?? snapshot.agents[0]?.id ?? null;
      }
      noteExpiring();
      if (!primed) {
        await tick();
        primed = true;
      }
    } catch (err) {
      if (snapshot) {
        daemonLost = true;
        if (lostAt == null) {
          lostAt = Date.now();
        }
      } else {
        error = err instanceof Error ? err.message : "The daemon is not reachable.";
      }
    }
  }

  async function simulate(forced = false): Promise<void> {
    busy = true;
    try {
      await emitPush(forced);
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : "The event was not accepted.";
    } finally {
      busy = false;
    }
  }

  async function ondecide(id: string, decision: Decision, reason?: string): Promise<boolean> {
    localDecisions.add(id);
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

  function showToast(text: string, review = false): void {
    toastText = text;
    toastReview = review;
  }

  function noteRemoteDecisions(approvals: Approval[]): void {
    if (decisionsSeeded && sheetVisible) {
      for (const [id, status] of previousStatus) {
        if (status !== "pending" || localDecisions.has(id)) {
          continue;
        }
        const next = approvals.find((approval) => approval.id === id);
        if (!next || next.status === "pending" || next.status === "expired") {
          continue;
        }
        const word = next.status === "approved" ? "Approved" : next.status === "denied" ? "Denied" : next.status;
        const ev = shortEventId(next.decision_event_id ?? "");
        showToast(`Decided on another device: ${word}${ev ? ` · ${ev}` : ""}`);
      }
    }
    previousStatus = new Map(approvals.map((approval) => [approval.id, approval.status]));
    decisionsSeeded = true;
  }

  function noteExpiring(): void {
    const approvals = snapshot?.approvals ?? [];
    for (const approval of approvals) {
      if (!isExpiringSoon(approval, now) || isDestructive(approval) || approval.status !== "pending") {
        continue;
      }
      if (!expiresFrozen.has(approval.id)) {
        const remaining = remainingMs(approval, now);
        if (remaining != null) {
          expiresFrozen.set(approval.id, formatExpires(remaining));
        }
      }
      if (!expiringToasted.has(approval.id)) {
        expiringToasted.add(approval.id);
        showToast(`${actionTitle(approval.action)} expires soon.`, true);
      }
    }
    if (!sheetVisible) {
      return;
    }
    const nextPins = [...expiredPinned];
    for (const approval of approvals) {
      if (approval.status !== "pending" || isDestructive(approval)) {
        continue;
      }
      if (carryDone.includes(approval.id) || doneIds.includes(approval.id)) {
        continue;
      }
      if (approval.expires_at != null && approval.expires_at <= now && !nextPins.includes(approval.id)) {
        nextPins.push(approval.id);
      }
    }
    if (nextPins.length !== expiredPinned.length) {
      expiredPinned = nextPins;
    }
  }

  function openReview(): void {
    if (!snapshot) {
      return;
    }
    doneIds = [...carryDone];
    sheetVisible = true;
    sheetOpen = true;
    void tick().then(() => {
      document.querySelector<HTMLElement>("[data-review-sheet]")?.focus();
    });
  }

  function requestClose(): void {
    sheetOpen = false;
  }

  function onSheetOutro(): void {
    if (expiredPinned.length > 0) {
      carryDone = [...new Set([...carryDone, ...expiredPinned])];
      expiredPinned = [];
    }
    sheetVisible = false;
    const resolve = resolveOutro;
    resolveOutro = null;
    resolve?.();
  }

  function waitOutro(): Promise<void> {
    return new Promise((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) {
          return;
        }
        settled = true;
        resolve();
      };
      resolveOutro = finish;
      window.setTimeout(finish, tokenMs("--dur-base", 240) + 180);
    });
  }

  function alignSourceAboveCard(id: string): number | null {
    const card = document.querySelector(`article[data-approval-id="${CSS.escape(id)}"]`);
    const source = document.querySelector(`[data-source-for="${CSS.escape(id)}"]`);
    const stage = document.querySelector(".stage");
    if (!(card instanceof HTMLElement) || !(source instanceof HTMLElement) || !(stage instanceof HTMLElement)) {
      return null;
    }
    const delta = source.getBoundingClientRect().bottom - (card.getBoundingClientRect().top - 18);
    if (Math.abs(delta) >= 1) {
      stage.scrollBy({ top: delta, behavior: "auto" });
    }
    return delta;
  }

  async function scrollSourceAboveCard(id: string): Promise<void> {
    alignSourceAboveCard(id);
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => resolve());
    });
    alignSourceAboveCard(id);
  }

  async function focusApproval(id: string): Promise<void> {
    const approval = (snapshot?.approvals ?? []).find((item) => item.id === id);
    if (!approval || isDestructive(approval)) {
      return;
    }
    const current = document.activeElement?.getAttribute("data-approval-id");
    if (!sheetVisible && !landing && current === id && focusedId === id) {
      return;
    }
    landing = true;
    if (threadAgentId !== approval.agent_id) {
      threadAgentId = approval.agent_id;
      switchAt = performance.now();
      await tick();
    }
    focusedId = id;
    await tick();
    if (sheetOpen) {
      const done = waitOutro();
      sheetOpen = false;
      await done;
      const start = performance.now();
      while (sheetVisible && performance.now() - start < 1000) {
        await new Promise((resolve) => {
          window.setTimeout(resolve, 16);
        });
      }
    }
    sheetVisible = false;
    const card = document.querySelector(`article[data-approval-id="${CSS.escape(id)}"]`);
    focusAt = performance.now();
    if (card instanceof HTMLElement) {
      card.focus({ preventScroll: true });
    }
    await tick();
    await scrollSourceAboveCard(id);
    landedAt = performance.now();
    landing = false;
  }

  function onQueueKey(event: KeyboardEvent): void {
    if (event.repeat || isTextEntry(event.target)) {
      return;
    }
    if (!event.altKey || event.metaKey || event.ctrlKey || event.shiftKey || event.key !== "ArrowDown") {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (waiting.length === 0) {
      sheetOpen = false;
      showToast("Nothing is waiting on you.");
      return;
    }
    const current = document.activeElement?.getAttribute("data-approval-id");
    const index = waiting.findIndex((approval) => approval.id === current);
    const next = index < 0 ? waiting[0] : waiting[Math.min(waiting.length - 1, index + 1)];
    if (!next) {
      return;
    }
    void focusApproval(next.id);
  }

  function scrimFade(
    _node: Element,
    _params: undefined,
    options: { direction: "in" | "out" | "both" },
  ): TransitionConfig {
    const intro = options.direction !== "out";
    return {
      duration: intro ? tokenMs("--dur-soft", 360) : tokenMs("--dur-base", 240),
      easing: intro ? tokenEase("--ease-out") : tokenEase("--ease-exit"),
      css: (t) => `opacity: ${t}`,
    };
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
    if (stream.some((row) => row.receipt?.id === target.id)) {
      return;
    }
    void onundo(target.id);
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

  $effect(() => {
    now;
    snapshot;
    sheetVisible;
    noteExpiring();
  });

  onMount(() => {
    void refresh();
    const clock = setInterval(() => {
      now = Date.now();
      noteExpiring();
    }, 200);
    const timer = setInterval(() => {
      void refresh();
    }, 1000);
    return () => {
      clearInterval(clock);
      clearInterval(timer);
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} onkeydowncapture={onQueueKey} />

{#snippet approvalSlot(approval: Approval)}
  <div class={["slot", approval.status === "pending" && "over"]} {@attach flipSlot}>
    {#key approval.id}
      <ApprovalCard
        approval={approval}
        busy={deciding}
        suspended={daemonLost}
        seenEnabled={approval.status === "pending" ? seenEnabled : true}
        landedAt={landing ? null : landedAt}
        hello={devWindowsHello}
        nextWaiting={approval.id === latestDecidedId && waiting.length > 0}
        shortcutTarget={approval.status === "pending"}
        ondecide={(decision, reason) => ondecide(approval.id, decision, reason)}
        onundo={() => onundo(approval.id)}
      />
    {/key}
  </div>
{/snippet}

<div
  class="well"
  style:--flat-radius={FLAT_RADIUS}
  data-thread-agent={threadAgentId ?? ""}
  data-switch-at={switchAt || undefined}
  data-focus-at={focusAt || undefined}
  data-sheet={sheetVisible ? "open" : "closed"}
  data-landing={landing ? "yes" : "no"}
  data-daemon={daemonLost ? "lost" : "ok"}
>
  <div class="shell">
    <header class="titlebar">
      <div class="wordmark">
        <span class="dots" aria-hidden="true"></span>
        <strong>dasdevbot</strong>
      </div>
      <div class="title-side">
        <button class="review-link" type="button" data-waiting-count={waiting.length} onclick={openReview}>
          Review <span class="n">{waiting.length}</span>
        </button>
        <div class="tray" data-tray>
          <svg class="tray-glyph" viewBox="0 0 16 16" aria-hidden="true">
            <rect x="2" y="3" width="12" height="10" rx="1.5" />
            <path d="M2 6.5h12" />
          </svg>
          {#if waiting.length > 0}
            <span class="tray-num">{waiting.length}</span>
          {/if}
        </div>
        <p class="status">
          {#if snapshot}
            {snapshot.role} · protocol {snapshot.protocol} · {snapshot.provider_detail} · sync {snapshot.sync}
          {:else}
            connecting
          {/if}
        </p>
      </div>
    </header>

    {#if error}
      <p class="banner" role="alert">{error}</p>
    {/if}

    <div class="body">
      <aside>
        <button class="wait-entry" type="button" onclick={openReview}>
          <span>Waiting on you · {waiting.length}</span>
          {#if !sheetVisible}
            <span class="wait-dots" aria-hidden="true">
              {#each waiting as item (item.id)}
                <span class="wdot" data-waiting-dot></span>
              {/each}
            </span>
          {/if}
        </button>
        <p class="section">Agents</p>
        {#if reviewer}
          <div class="agent" data-status={reviewer.status}>
            <div class="agent-row">
              <h2>{reviewer.name}</h2>
              {#if reviewer.status === "working"}
                <span class="agent-status">{reviewer.status}</span>
              {:else if filingUndo && filingSeconds > 0}
                <span class="agent-status">filing · undo {filingSeconds}s</span>
              {:else if reviewer.status === "blocked" && !filingUndo}
                <span class="agent-status">{waiting.length > 0 ? "waiting" : reviewer.status}</span>
              {/if}
            </div>
            <p class="project">{reviewer.project}</p>
            <p class="budget">{reviewer.tokens_spent} / {reviewer.token_cap} tok</p>
            <p class="persona">{reviewer.persona.trim()}</p>
          </div>
        {:else}
          <p class="muted">No agents stored.</p>
        {/if}

        <div class="sim-row">
          <button class="simulate" type="button" disabled={busy} onclick={() => void simulate(false)}>
            {busy ? "Waking Reviewer" : "Simulate repo.push"}
          </button>
          <button class="simulate" type="button" disabled={busy} onclick={() => void simulate(true)}>
            {busy ? "Waking Reviewer" : "Simulate force push"}
          </button>
        </div>
        <p class="hint">Reviewer is a stored row. It runs only when this event wakes it.</p>
      </aside>

      <main>
        <p class="section">Stream</p>
        <div class="stage" {@attach pinOverlay}>
          <ol class="stream">
            {#each stream as row (row.id)}
              {@const linked = approvalByKey[row.key] ?? null}
              {@const destructive = linked != null && linked.status === "pending" && isDestructive(linked)}
              {@const filed = linked != null && linked.status !== "pending"}
              {@const showCard = linked != null && !destructive && (filed || linked.id === pending?.id)}
              <li class={filed ? "slot-row" : "event"} animate:stepRows>
                {#if destructive && linked}
                  <div class="row policy" data-destructive={linked.id} in:arrive|global={{ play: primed }}>
                    <time class="when" datetime={row.iso || undefined}>{row.when}</time>
                    <span class="dash" aria-hidden="true"></span>
                    <div class="copy">
                      <p class="who">Destructive actions are off in this build.</p>
                      <p class="detail">{linked.draft}</p>
                    </div>
                  </div>
                {:else}
                  {#if !filed}
                    <div
                      class="row"
                      title={row.title}
                      data-source-for={linked && linked.status === "pending" ? linked.id : undefined}
                      in:arrive|global={{ play: primed }}
                    >
                      <time class="when" datetime={row.iso || undefined}>{row.when}</time>
                      <span class="disc" aria-hidden="true">{row.mark}</span>
                      <div class="copy">
                        <p class="who">{row.who}</p>
                        <p class="detail">{row.detail}</p>
                      </div>
                    </div>
                  {/if}
                  {#if showCard && linked}
                    {@render approvalSlot(linked)}
                  {/if}
                {/if}
              </li>
            {/each}
          </ol>
          {#each blocked as approval (approval.id)}
            {#if !stream.some((row) => row.key === `approval-requested:${approval.id}`)}
              <div class="row policy orphan" data-destructive={approval.id}>
                <time class="when">{formatStreamTime(approval.created_at)}</time>
                <span class="dash" aria-hidden="true"></span>
                <div class="copy">
                  <p class="who">Destructive actions are off in this build.</p>
                  <p class="detail">{approval.draft}</p>
                </div>
              </div>
            {/if}
          {/each}
          {#if pending && !pendingAnchored}
            {@render approvalSlot(pending)}
          {:else if !pending && !anyReceipt && blocked.length === 0}
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
    </div>
  </div>

  {#if toastText}
    <div class="toast" role="status" data-toast>
      <p>{toastText}</p>
      {#if toastReview}
        <button type="button" onclick={openReview}>Review</button>
      {/if}
    </div>
  {/if}

  {#if sheetOpen}
    <div class="scrim" transition:scrimFade></div>
    <ReviewSheet
      rows={sheetRows}
      done={doneRows}
      summary={queueSummary(waiting.length)}
      {daemonLine}
      onopen={(id) => void focusApproval(id)}
      onclose={requestClose}
      onsettled={onSheetOutro}
    />
  {/if}
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
    border-radius: var(--flat-radius);
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

  .title-side {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
  }

  .review-link {
    border: 0;
    background: none;
    color: var(--ink-2);
    font-size: var(--t-meta);
    font-weight: var(--w-medium);
    cursor: pointer;
    padding: 6px 8px;
    border-radius: var(--r-sm);
  }

  .review-link .n {
    color: var(--ink-1);
    font-weight: var(--w-semibold);
    font-variant-numeric: tabular-nums;
  }

  .review-link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .tray {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--ink-1);
  }

  .tray-glyph {
    width: 16px;
    height: 16px;
  }

  .tray-glyph rect,
  .tray-glyph path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .tray-num {
    font-family: var(--font-machine);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    color: var(--ink-1);
    font-variant-numeric: tabular-nums;
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

  .wait-entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    margin-bottom: var(--s-3);
    padding: 8px 2px;
    border: 0;
    border-bottom: 1px solid var(--hairline);
    border-radius: 0;
    background: none;
    color: var(--ink-1);
    font-size: var(--t-body);
    line-height: var(--lh-body);
    text-align: left;
    cursor: pointer;
  }

  .wait-entry:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .wait-dots {
    display: inline-flex;
    gap: 4px;
  }

  .wdot {
    width: 7px;
    height: 7px;
    border-radius: var(--r-pill);
    background: var(--risk-external);
  }

  .agent {
    position: relative;
    overflow: hidden;
    padding: var(--s-3);
    border: 1px solid var(--hairline);
    border-radius: var(--flat-radius);
    background: var(--paper-raised);
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

  .project,
  .budget,
  .hint,
  .muted,
  .persona {
    color: var(--ink-2);
    font-size: var(--t-meta);
  }

  .agent-status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ink-3);
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

  .sim-row {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    margin-top: var(--s-4);
  }

  .simulate {
    width: 100%;
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
    max-height: calc(100dvh - 240px);
    overflow: auto;
  }

  .stream {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0;
    margin: 0;
    padding: 0 0 520px;
  }

  .dash {
    width: 12px;
    height: 1.5px;
    justify-self: center;
    background: var(--ink-1);
  }

  .row.policy .who {
    color: var(--ink-1);
  }

  .orphan {
    margin-top: var(--s-2);
  }

  .event,
  .slot-row {
    list-style: none;
    border-radius: var(--flat-radius);
    background: none;
    box-shadow: none;
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
  }

  .detail {
    margin-top: 2px;
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
    overflow-wrap: anywhere;
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
  }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: rgb(var(--shade) / 0.1);
  }

  .toast {
    position: fixed;
    z-index: 30;
    left: 0;
    right: 0;
    bottom: 24px;
    margin: 0 auto;
    width: max-content;
    max-width: calc(100% - 48px);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-radius: var(--r-pill);
    background: var(--glass-fill);
    -webkit-backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturate));
    box-shadow: var(--glass-edge), var(--shadow-float);
    color: var(--ink-1);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .toast button {
    border: 0;
    background: none;
    box-shadow: none;
    color: var(--accent);
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    cursor: pointer;
    padding: 0;
  }

  .toast button:hover,
  .toast button:active {
    transform: none;
    box-shadow: none;
  }

  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    .toast {
      background: var(--glass-fill-solid);
    }
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
