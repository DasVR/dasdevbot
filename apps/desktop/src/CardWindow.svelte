<script lang="ts">
  // The `card` Tauri window. It is the only window with the sign_decision and
  // undo_decision capabilities, so this is where a card is approved, denied or
  // undone. The main window can only ask to show it.
  import { onMount } from "svelte";
  import ApprovalCard from "./lib/ApprovalCard.svelte";
  import { decide, getSnapshot, undo, type Approval, type Decision } from "./lib/api";
  import { startPolling } from "./lib/poll";

  let approvals = $state<Approval[]>([]);
  let error = $state<string | null>(null);
  let deciding = $state(false);
  let now = $state(Date.now());

  // The open card, or the one whose undo window is still running.
  const current = $derived(
    approvals.find((approval) => approval.status === "pending") ??
      approvals.find(
        (approval) =>
          !approval.committed &&
          (approval.status === "approved" || approval.status === "denied") &&
          approval.undo_until != null &&
          approval.undo_until > now,
      ) ??
      null,
  );

  async function refresh(): Promise<void> {
    try {
      approvals = (await getSnapshot()).approvals;
      error = null;
    } catch (err) {
      error = err instanceof Error ? err.message : "The daemon did not answer.";
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

  onMount(() => {
    const stopPolling = startPolling(refresh, 1000);
    const clock = setInterval(() => {
      now = Date.now();
    }, 200);
    return () => {
      clearInterval(clock);
      stopPolling();
    };
  });
</script>

<main class="card-window">
  {#if error}
    <p class="note" role="status">{error}</p>
  {/if}
  {#if current}
    {#key current.id}
      <ApprovalCard
        approval={current}
        busy={deciding}
        shortcutTarget={current.status === "pending"}
        ondecide={(decision, reason) => ondecide(current.id, decision, reason)}
        onundo={() => onundo(current.id)}
      />
    {/key}
  {:else}
    <p class="note">No card is waiting.</p>
  {/if}
</main>

<style>
  .card-window {
    box-sizing: border-box;
    min-height: 100vh;
    padding: var(--s-5);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-3);
    background: var(--paper-base);
  }

  .note {
    margin: 0;
    font: var(--w-regular) var(--t-body) / var(--lh-body) var(--font-ui);
    color: var(--ink-2);
  }
</style>
