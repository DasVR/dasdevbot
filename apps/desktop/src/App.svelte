<script lang="ts">
  import { onMount } from "svelte";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import {
    decide,
    emitPush,
    formatUsd,
    getSnapshot,
    isTextEntry,
    undo,
    type Approval,
    type Decision,
    type Snapshot,
  } from "./lib/api";

  let snapshot = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let deciding = $state(false);

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
    return [...events].reverse().map((event) => {
      const source = event.source.trim();
      return {
        id: event.id,
        mark: source.charAt(0).toUpperCase() || "·",
        who: source ? `${source} · ${event.kind}` : event.kind,
        body: `${event.hlc} · ${event.id}`,
      };
    });
  });
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
              <li class="ev">
                <span class="disc" aria-hidden="true">{row.mark}</span>
                <div class="bubble">
                  <p class="who">{row.who}</p>
                  <p class="ev-body">{row.body}</p>
                </div>
              </li>
            {/each}
          </ol>
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
  .project,
  .ev-body {
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
    gap: 12px;
    margin: 0;
    padding: 0;
  }

  .ev {
    display: flex;
    gap: 12px;
    align-items: flex-start;
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

  .bubble {
    flex: 1;
    min-width: 0;
    background: var(--convex), var(--paper-raised);
    border: 1px solid var(--hairline);
    border-radius: var(--r-lg);
    padding: 12px 16px;
    box-shadow: var(--shadow-puff);
  }

  .who {
    color: var(--ink-3);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
  }

  .ev-body {
    margin-top: 4px;
    color: var(--ink-1);
    font-size: 12px;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .slot {
    margin-top: 14px;
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
