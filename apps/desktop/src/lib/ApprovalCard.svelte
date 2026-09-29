<script lang="ts">
  import { effectLabel, formatUsd, parseEffect, type Approval, type Decision } from "./api";

  interface Props {
    approval: Approval;
    busy?: boolean;
    ondecide?: (decision: Decision) => void;
  }

  let { approval, busy = false, ondecide }: Props = $props();

  const effect = $derived(parseEffect(approval.effect_class));
  const label = $derived(effect ? effectLabel(effect) : approval.effect_class);
  const risk = $derived(effect ?? "unknown");
  const pending = $derived(approval.status === "pending");
</script>

<article class="card" data-risk={risk}>
  <header class="card-head">
    <div>
      <p class="kicker">{approval.agent_name}</p>
      <h2>Post a PR comment</h2>
    </div>
    <span class="chip" data-risk={risk}>{label}</span>
  </header>

  <p class="purpose">{approval.purpose}</p>

  <section class="block">
    <h3>Evidence</h3>
    <pre>{approval.evidence}</pre>
  </section>

  <section class="block">
    <h3>Draft</h3>
    <p class="draft">{approval.draft}</p>
  </section>

  <p class="meta">
    {approval.provider} · {approval.model} · {approval.usage_kind} · {approval.input_tokens} in / {approval.output_tokens} out · {formatUsd(approval.micro_usd)}
  </p>

  {#if pending}
    <div class="actions">
      <button class="primary" type="button" disabled={busy} onclick={() => ondecide?.("approve")}>
        Approve
      </button>
      <button class="ghost" type="button" disabled={busy} onclick={() => ondecide?.("deny")}>
        Deny
      </button>
    </div>
  {:else}
    <p class="settled">
      {approval.status === "approved" ? "Approved" : "Denied"}. The decision is in the event log. Nothing was posted.
    </p>
  {/if}
</article>

<style>
  .card {
    background: var(--nil-raised);
    border: 1px solid var(--nil-line);
    border-left-width: 3px;
    border-left-color: var(--nil-line-hot);
    border-radius: var(--r-card);
    box-shadow: var(--lift-1);
    padding: var(--s-4);
  }

  .card[data-risk="external"] {
    border-left-color: var(--sev-high);
  }

  .card[data-risk="write_local"] {
    border-left-color: var(--sev-medium);
  }

  .card[data-risk="destructive"] {
    border-left-color: var(--sev-critical);
  }

  .card-head {
    display: flex;
    justify-content: space-between;
    gap: var(--s-3);
    align-items: flex-start;
  }

  .kicker {
    color: var(--nil-ink-3);
    font-size: var(--t-micro);
    letter-spacing: var(--track-tick);
    text-transform: uppercase;
  }

  h2 {
    font-size: var(--t-lead);
    font-weight: 600;
    letter-spacing: var(--track-tight);
    line-height: var(--lh-tight);
  }

  .chip {
    flex-shrink: 0;
    border-radius: var(--r-chip);
    padding: 2px 8px;
    font-size: var(--t-micro);
    letter-spacing: var(--track-tick);
    text-transform: uppercase;
    color: var(--nil-ink-2);
    background: var(--sev-info-bg);
  }

  .chip[data-risk="external"] {
    color: var(--sev-high);
    background: var(--sev-high-bg);
  }

  .chip[data-risk="write_local"] {
    color: var(--sev-medium);
    background: var(--sev-medium-bg);
  }

  .chip[data-risk="destructive"] {
    color: var(--sev-critical);
    background: var(--sev-critical-bg);
  }

  .purpose {
    margin-top: var(--s-3);
    color: var(--nil-ink-2);
  }

  .block {
    margin-top: var(--s-4);
  }

  h3 {
    color: var(--nil-ink-3);
    font-size: var(--t-micro);
    font-weight: 500;
    letter-spacing: var(--track-tick);
    text-transform: uppercase;
    margin-bottom: var(--s-2);
  }

  pre {
    font-family: var(--font-machine);
    font-size: var(--t-meta);
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--nil-ink);
    background: var(--nil-panel);
    border-radius: var(--r-field);
    padding: var(--s-3);
  }

  .draft {
    white-space: pre-wrap;
  }

  .meta {
    margin-top: var(--s-3);
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    color: var(--nil-ink-3);
  }

  .actions {
    display: flex;
    gap: var(--s-2);
    margin-top: var(--s-4);
  }

  button {
    height: 32px;
    padding: 0 var(--s-3);
    border-radius: var(--r-field);
    font: 500 var(--t-meta) / 1 var(--font-ui);
    cursor: pointer;
    transition:
      transform var(--dur-flip) var(--ease-out),
      border-color var(--dur-flip) var(--ease-out),
      background-color var(--dur-flip) var(--ease-out),
      box-shadow var(--dur-enter) var(--ease-out);
  }

  button:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: var(--lift-1);
  }

  button:active:not(:disabled) {
    transform: scale(0.955);
    transition-duration: 70ms;
    box-shadow: none;
  }

  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .primary {
    background: var(--nil-panel);
    color: var(--nil-ink);
    border: 1px solid var(--nil-line-hot);
  }

  .ghost {
    background: transparent;
    color: var(--nil-ink-2);
    border: 1px solid var(--nil-line);
  }

  .settled {
    margin-top: var(--s-4);
    color: var(--nil-ink);
    font-weight: 500;
  }
</style>
