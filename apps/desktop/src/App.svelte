<script lang="ts">
  import { onMount } from "svelte";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import { decide, emitPush, formatUsd, getSnapshot, type Decision, type Snapshot } from "./lib/api";

  let snapshot = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let deciding = $state(false);

  const reviewer = $derived(snapshot?.agents.find((agent) => agent.id === "reviewer") ?? null);
  const pending = $derived(snapshot?.approvals.find((approval) => approval.status === "pending") ?? null);
  const history = $derived(snapshot?.approvals.filter((approval) => approval.status !== "pending") ?? []);

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

  async function ondecide(id: string, decision: Decision): Promise<void> {
    deciding = true;
    try {
      await decide(id, decision);
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : "The decision was not recorded.";
    } finally {
      deciding = false;
    }
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

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
        <p class="section">Approval</p>
        {#if pending}
          <ApprovalCard approval={pending} busy={deciding} ondecide={(decision) => void ondecide(pending.id, decision)} />
        {:else if history.length > 0}
          <ApprovalCard approval={history[0]} />
        {:else}
          <div class="empty">
            <h2>Nothing is waiting.</h2>
            <p>A push wakes Reviewer. The turn drafts a comment and asks before any external effect. Approving records the decision and does not post it.</p>
          </div>
        {/if}

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

        <p class="section ledger-head">Event log</p>
        <ol class="log">
          {#each snapshot?.events ?? [] as event (event.id)}
            <li>
              <span>{event.hlc}</span>
              <span>{event.kind}</span>
              <span>{event.id}</span>
            </li>
          {/each}
        </ol>
      </main>
    </div>
  </div>
</div>

<style>
  .well {
    min-height: 100%;
    padding: var(--s-5);
    background:
      var(--well-depth),
      var(--nil-void);
  }

  .shell {
    max-width: 1120px;
    margin: 0 auto;
    background-color: var(--nil-panel);
    background-image: var(--panel-depth);
    border: 1px solid var(--nil-line);
    border-radius: var(--r-window);
    box-shadow: var(--lift-2);
    overflow: hidden;
  }

  .titlebar {
    display: flex;
    justify-content: space-between;
    gap: var(--s-4);
    align-items: center;
    padding: var(--s-3) var(--s-4);
    border-bottom: 1px solid var(--nil-line);
  }

  .wordmark {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    font-weight: 600;
    letter-spacing: var(--track-tight);
  }

  .dots {
    width: 42px;
    height: 10px;
    background:
      radial-gradient(circle at 5px 5px, var(--nil-ink-4) 4px, transparent 4.5px),
      radial-gradient(circle at 21px 5px, var(--nil-ink-4) 4px, transparent 4.5px),
      radial-gradient(circle at 37px 5px, var(--nil-ink-4) 4px, transparent 4.5px);
  }

  .status,
  .budget,
  .log,
  .ledger,
  .project {
    font-family: var(--font-machine);
  }

  .status {
    color: var(--nil-ink-3);
    font-size: var(--t-micro);
    text-align: right;
  }

  .banner {
    margin: var(--s-3) var(--s-4) 0;
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-field);
    color: var(--sev-critical);
    background: var(--sev-critical-bg);
    border: 1px solid color-mix(in oklab, var(--sev-critical) 35%, transparent);
  }

  .body {
    display: grid;
    grid-template-columns: 280px 1fr;
    min-height: 640px;
  }

  aside {
    padding: var(--s-4);
    border-right: 1px solid var(--nil-line);
  }

  main {
    padding: var(--s-4);
  }

  .section {
    color: var(--nil-ink-3);
    font-size: var(--t-micro);
    letter-spacing: var(--track-tick);
    text-transform: uppercase;
    margin-bottom: var(--s-3);
  }

  .agent {
    position: relative;
    overflow: hidden;
    padding: var(--s-3);
    border: 1px solid var(--nil-line);
    border-radius: var(--r-card);
    background: var(--nil-void);
  }

  .agent-row {
    display: flex;
    justify-content: space-between;
    gap: var(--s-2);
    align-items: baseline;
  }

  h2 {
    font-size: var(--t-lead);
    font-weight: 600;
  }

  .agent-status,
  .project,
  .budget,
  .hint,
  .muted,
  .persona {
    color: var(--nil-ink-3);
    font-size: var(--t-meta);
  }

  .persona {
    margin-top: var(--s-2);
    display: -webkit-box;
    -webkit-line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .budget {
    margin-top: var(--s-2);
    color: var(--nil-ink-2);
  }

  .scan {
    margin-top: var(--s-3);
    height: 2px;
    overflow: hidden;
    background: var(--nil-line);
  }

  .scan::after {
    content: "";
    display: block;
    height: 100%;
    width: 35%;
    background: var(--nil-ink);
    animation: scan 1.1s linear infinite;
  }

  .simulate {
    width: 100%;
    margin-top: var(--s-4);
    height: 32px;
    border-radius: var(--r-field);
    border: 1px solid var(--nil-line-hot);
    background: var(--nil-raised);
    color: var(--nil-ink);
    font: 500 var(--t-meta) / 1 var(--font-ui);
    cursor: pointer;
  }

  .simulate:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .hint {
    margin-top: var(--s-2);
  }

  .empty {
    padding: var(--s-5);
    border: 1px solid var(--nil-line);
    border-radius: var(--r-card);
    background: var(--nil-raised);
  }

  .empty p,
  .muted {
    margin-top: var(--s-2);
    color: var(--nil-ink-2);
  }

  .ledger-head {
    margin-top: var(--s-5);
  }

  .ledger {
    list-style: none;
    display: grid;
    gap: var(--s-2);
  }

  .ledger li {
    font-size: var(--t-meta);
    color: var(--nil-ink);
  }

  .ledger span,
  .log {
    display: block;
    color: var(--nil-ink-3);
    font-size: var(--t-micro);
  }

  .log {
    list-style: none;
    display: grid;
    gap: 4px;
  }

  .log li {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) 0.8fr minmax(0, 1.2fr);
    gap: var(--s-3);
  }

  .log span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--nil-ink-2);
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
      border-bottom: 1px solid var(--nil-line);
    }

    .titlebar {
      flex-direction: column;
      align-items: flex-start;
    }

    .status {
      text-align: left;
    }

    .log li {
      grid-template-columns: 1fr;
    }
  }
</style>
