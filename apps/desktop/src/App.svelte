<script lang="ts">
  import { tick } from "svelte";
  import { flip, type AnimationConfig } from "svelte/animate";
  import { linear } from "svelte/easing";
  import type { TransitionConfig } from "svelte/transition";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import Curl from "./lib/Curl.svelte";
  import FirstRun from "./lib/FirstRun.svelte";
  import FocusIcon from "./lib/FocusIcon.svelte";
  import GlyphSlot from "./lib/GlyphSlot.svelte";
  import { tokenEase, tokenMs } from "./lib/cssTokens";
  import {
    dayPart,
    doneToday,
    greetingHeading,
    greetingLine,
    greetingSeen,
    markGreetingSeen,
    modeReturnSkipsGreeting,
  } from "./lib/greeting";
  import { isFirstRunComplete, landingCopy, loadRecord } from "./lib/firstRun/model";
  import { askAfterLine, loadPermissionMemory, savePermissionMemory } from "./lib/firstRun/permissions";
  import { micPort, notificationPort } from "./lib/firstRun/ports";
  import { QUIET_LINE_PATH } from "./lib/pen";
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
  let onboarded = $state(isFirstRunComplete());
  let landedNow = false;
  let greetingOn = $state(false);
  let greetingName = $state("");
  let underlineOn = $state(false);
  const remembered = loadPermissionMemory();
  let notificationsAsked = $state(remembered.notifications);
  let micAsked = $state(remembered.mic);
  let permissionLine = $state<string | null>(null);
  let unfocused = $state(false);
  let permissionBusy = false;

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
        receipt: receipts[event.idempotency_key] ?? null,
        pendingHere: pendingKey !== "" && event.idempotency_key === pendingKey,
      };
    });
  });
  const pendingAnchored = $derived(stream.some((row) => row.pendingHere));
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
  const waitingCount = $derived(
    (snapshot?.approvals ?? []).filter((approval) => approval.status === "pending").length,
  );
  const finishedToday = $derived(doneToday(snapshot?.approvals ?? [], now));
  const greetPart = $derived(dayPart(new Date(now)));
  const greetTitle = $derived(greetingName ? greetingHeading(greetPart, greetingName) : "");
  const greetSub = $derived(greetingLine(greetPart, waitingCount, finishedToday));
  const watchedRepo = $derived(loadRecord()?.repo ?? reviewer?.project ?? "DasVR/NIL");
  const filingSeconds = $derived(
    filingUndo?.undo_until == null
      ? 0
      : Math.max(0, Math.ceil((filingUndo.undo_until - now) / 1000)),
  );

  async function refresh(): Promise<void> {
    try {
      snapshot = await getSnapshot();
      error = null;
      await tick();
      if (!primed) {
        primed = true;
      }
      await askNotifications();
    } catch (err) {
      error = err instanceof Error ? err.message : "The daemon is not reachable.";
    }
  }

  function paintFrame(): Promise<void> {
    return new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
  }

  function rememberPermissions(): void {
    savePermissionMemory({ notifications: notificationsAsked, mic: micAsked });
  }

  async function askNotifications(): Promise<void> {
    if (!onboarded || notificationsAsked || permissionBusy || permissionLine) {
      return;
    }
    if ((document.visibilityState === "visible" && !unfocused) || !pending) {
      return;
    }
    notificationsAsked = true;
    rememberPermissions();
    permissionBusy = true;
    try {
      await askAfterLine({
        line: "Want a Review banner when something waits on you?",
        show: async (line) => {
          permissionLine = line;
          await tick();
        },
        hide: () => {
          permissionLine = null;
        },
        paint: paintFrame,
        request: () => notificationPort().request(),
      });
    } finally {
      permissionBusy = false;
      permissionLine = null;
    }
  }

  async function onTalk(): Promise<void> {
    if (micAsked || permissionBusy || permissionLine) {
      return;
    }
    micAsked = true;
    rememberPermissions();
    permissionBusy = true;
    try {
      await askAfterLine({
        line: "The mic stays off until you hold to talk.",
        show: async (line) => {
          permissionLine = line;
          await tick();
        },
        hide: () => {
          permissionLine = null;
        },
        paint: paintFrame,
        request: () => micPort().request(),
      });
    } finally {
      permissionBusy = false;
      permissionLine = null;
    }
  }

  function openGreeting(): void {
    if (!onboarded || landedNow || greetingOn || modeReturnSkipsGreeting()) {
      return;
    }
    const name = loadRecord()?.login;
    if (!name || greetingSeen(new Date())) {
      return;
    }
    markGreetingSeen(new Date());
    greetingName = name;
    greetingOn = true;
  }

  function dismissGreeting(): void {
    greetingOn = false;
  }

  function completeFirstRun(): void {
    landedNow = true;
    onboarded = true;
  }

  async function simulate(forced = false): Promise<void> {
    dismissGreeting();
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
    if (!onboarded || event.repeat || isTextEntry(event.target)) {
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
    if (!onboarded) {
      return;
    }
    openGreeting();
    void refresh();
    const clock = setInterval(() => {
      now = Date.now();
    }, 200);
    const timer = setInterval(() => {
      void refresh();
    }, 1000);
    const onBlur = () => {
      unfocused = true;
      void askNotifications();
    };
    const onFocus = () => {
      unfocused = false;
    };
    const onVisibility = () => {
      void askNotifications();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(clock);
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", onFocus);
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} />

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

{#if !onboarded}
  <FirstRun onDone={completeFirstRun} />
{:else}
<div class="well" style:--flat-radius={FLAT_RADIUS} data-mode="here" data-watching="on">
  <div class="shell">
    <header class="titlebar">
      <div class="wordmark">
        <span class="dots" aria-hidden="true"></span>
        <strong>dasdevbot</strong>
      </div>
      <p class="mode">
        <FocusIcon />
        <span>Here</span>
      </p>
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
              <button class="agent-name" type="button" onclick={dismissGreeting}>{reviewer.name}</button>
              {#if reviewer.status === "working"}
                <span class="agent-status">
                  <svg class="running-trace" data-running-trace viewBox="0 0 72 16" aria-hidden="true">
                    <path class="pen" pathLength="1" d="M2 9 C16 6 28 12 44 8 C54 6 62 10 70 8" />
                  </svg>
                  {reviewer.status}
                </span>
              {:else if filingUndo && filingSeconds > 0}
                <span class="agent-status">filing · undo {filingSeconds}s</span>
              {:else if reviewer.status === "blocked" && !filingUndo}
                <span class="agent-status need">
                  <span class="need-dot" aria-hidden="true"></span>
                  {reviewer.status}
                </span>
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
        <button class="talk" type="button" onclick={() => void onTalk()}>Hold to talk</button>
      </aside>

      <main>
        {#if greetingOn}
          <section class="greet" data-greeting>
            <GlyphSlot>
              {#snippet glyph()}
                <Curl blink />
              {/snippet}
            </GlyphSlot>
            <div>
              <h2 class="greet-title">{greetTitle}</h2>
              <p class="greet-sub" data-greeting-sub>{greetSub}</p>
            </div>
          </section>
        {/if}
        <p class="section">Stream</p>
        {#if permissionLine}
          <p class="why" data-permission-line aria-live="polite">{permissionLine}</p>
        {/if}
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
          {:else if stream.length === 0}
            <div class="empty-state" data-landing>
              {#if !greetingOn}
                <div class="landing-row">
                    <GlyphSlot>
                      {#snippet glyph()}
                        <Curl blink onLanded={() => (underlineOn = true)} />
                      {/snippet}
                    </GlyphSlot>
                  <div>
                    <h1 class="empty-title">Nothing waiting on you</h1>
                    <svg class="quiet-line" viewBox="0 0 104 10" aria-hidden="true">
                      <path
                        class="pen"
                        class:draw={underlineOn}
                        data-empty-underline
                        pathLength="1"
                        d={QUIET_LINE_PATH}
                      />
                    </svg>
                  </div>
                </div>
              {/if}
              <p class="quiet-copy">{landingCopy(watchedRepo)}</p>
            </div>
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
{/if}

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
    justify-content: flex-start;
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

  .mode {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    margin-right: auto;
    margin-left: var(--s-4);
    color: var(--ink-1);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
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
    border-radius: var(--flat-radius);
    background: var(--paper-raised);
  }

  .agent-row {
    display: flex;
    justify-content: space-between;
    gap: var(--s-2);
    align-items: baseline;
  }

  .agent-name {
    border: 0;
    padding: 0;
    background: none;
    color: inherit;
    font: inherit;
    font-size: var(--t-lead);
    line-height: var(--lh-lead);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
    cursor: pointer;
    text-align: left;
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

  .need-dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: var(--r-pill);
    background: var(--risk-external);
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

  .running-trace {
    width: 36px;
    height: 12px;
    overflow: visible;
  }

  .running-trace .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: var(--pen-width);
    stroke-linecap: round;
    stroke-dasharray: 1;
    stroke-dashoffset: 0;
    animation: trace-run var(--dur-trace-read) linear infinite;
  }

  .greet {
    display: flex;
    align-items: flex-start;
    gap: var(--s-4);
    margin-bottom: var(--s-6);
  }

  .greet-title {
    font-size: var(--t-display);
    line-height: var(--lh-display);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
    color: var(--ink-1);
  }

  .greet-sub {
    margin-top: var(--s-1);
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-3);
    margin-top: var(--s-5);
    max-width: 40rem;
  }

  .landing-row {
    display: flex;
    align-items: flex-start;
    gap: var(--s-4);
  }

  .empty-title {
    font-size: var(--t-display);
    line-height: var(--lh-display);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
    color: var(--ink-1);
  }

  .quiet-copy {
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .quiet-line {
    width: 168px;
    height: 10px;
    margin-top: calc(var(--s-1) * -1);
    overflow: visible;
  }

  .empty-state .pen {
    fill: none;
    stroke: var(--pen);
    stroke-width: var(--pen-width);
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
  }

  .empty-state .draw {
    animation: draw-quiet var(--dur-draw) var(--ease-draw) 1 both;
  }

  @keyframes trace-run {
    from {
      stroke-dashoffset: 1;
    }
    to {
      stroke-dashoffset: 0;
    }
  }

  .why {
    margin: 0 0 var(--s-3);
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .talk {
    margin-top: var(--s-4);
    width: 100%;
    height: 40px;
    padding: 0 var(--s-4);
    border-radius: var(--r-md);
    border: 1.5px solid var(--ink-1);
    background: var(--paper-raised);
    color: var(--ink-1);
    font-size: var(--t-meta);
    font-weight: var(--w-semibold);
    cursor: pointer;
  }

  .talk:active {
    background: var(--paper-sunken);
  }

  .talk:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .empty-state .pen,
    .running-trace .pen {
      animation: none;
      stroke-dashoffset: 0;
    }

    .talk:active {
      transition: none;
    }
  }

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
