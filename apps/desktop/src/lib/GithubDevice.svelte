<script lang="ts">
  import { loginName } from "./firstRun/model";
  import { postHandoff, type HandoffOutcome } from "./firstRun/ports";

  /** Dev stand-in for the account the system browser already signed in. Not a token. */
  const DEV_LOGIN = "arriq";

  const requested = new URLSearchParams(window.location.search).get("login");
  const account = loginName(requested) ?? DEV_LOGIN;

  function finish(outcome: HandoffOutcome): void {
    postHandoff(outcome, outcome === "connected" ? account : undefined);
    window.close();
  }
</script>

<main class="page">
  <p class="brand">dasdevbot</p>
  <h1>Finish in the browser</h1>
  <p class="lead">This page stands in for GitHub. Nothing here is a token.</p>
  <p class="lead">Signed in as {account}.</p>
  <p class="code"><span>Device code</span> DEV-HAND</p>
  <div class="actions">
    <button class="primary" type="button" onclick={() => finish("connected")}>Continue</button>
    <button class="quiet" type="button" onclick={() => finish("cancelled")}>Cancel</button>
  </div>
</main>

<style>
  .page {
    min-height: 100%;
    padding: var(--s-8);
    background: var(--paper-base);
    color: var(--ink-1);
  }

  .brand {
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  h1 {
    margin-top: var(--s-6);
    font-size: var(--t-display);
    line-height: var(--lh-display);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  .lead {
    margin-top: var(--s-4);
    max-width: 36rem;
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .code {
    margin-top: var(--s-5);
    font-family: var(--font-machine);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    color: var(--ink-1);
  }

  .code span {
    display: block;
    margin-bottom: var(--s-1);
    color: var(--ink-3);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--s-4);
    margin-top: var(--s-6);
  }

  .primary,
  .quiet {
    height: 40px;
    padding: 0 var(--s-4);
    border-radius: var(--r-md);
    font-size: var(--t-body);
    font-weight: var(--w-semibold);
    cursor: pointer;
  }

  .primary {
    border: 0;
    background: var(--ink-1);
    color: var(--paper-raised);
  }

  .primary:active {
    background: var(--ink-press);
  }

  .quiet {
    border: 0;
    background: transparent;
    color: var(--ink-1);
  }

  .quiet:active {
    background: var(--paper-sunken);
  }
</style>
