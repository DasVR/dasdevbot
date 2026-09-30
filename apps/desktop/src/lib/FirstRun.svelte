<script lang="ts">
  import { onMount } from "svelte";
  import Curl from "./Curl.svelte";
  import GlyphSlot from "./GlyphSlot.svelte";
  import PenCheckbox from "./PenCheckbox.svelte";
  import {
    GITHUB_LEAD,
    REPO_LEAD,
    canPick,
    connectedCopy,
    emptyRecord,
    emptyRepoCopy,
    fixAccessHref,
    helperMissCopy,
    helperSkipCopy,
    highlightParts,
    loadRecord,
    normalizeAddress,
    previousStep,
    saveRecord,
    stepNumber,
    stepTitle,
    visibleRepos,
    type FirstRunRecord,
    type FirstRunStep,
    type RepoChoice,
  } from "./firstRun/model";
  import {
    HANDOFF_STORAGE_KEY,
    browserPort,
    consumeHandoff,
    devicePageUrl,
    helperPort,
    listenForHandoff,
    repoCatalog,
    type HandoffListen,
    type HandoffPayload,
  } from "./firstRun/ports";

  interface Props {
    onDone: () => void;
  }

  let { onDone }: Props = $props();

  const saved = loadRecord();
  const repos = repoCatalog();

  let phase = $state<"checking" | "flow">(saved ? "flow" : "checking");
  let step = $state<FirstRunStep>(saved?.step ?? "helper");
  let helperSkipped = $state(saved?.helperSkipped ?? false);
  let github = $state(saved?.github ?? "idle");
  let handoffUrl = $state<string | null>(saved?.handoffUrl ?? null);
  let login = $state<string | null>(saved?.login ?? null);
  let picked = $state<string | null>(saved?.repo ?? null);
  let address = $state(saved?.address ?? emptyRecord().address);
  let elsewhere = $state(saved?.elsewhere ?? false);
  let askPost = $state(saved?.askPost ?? true);
  let askWrite = $state(saved?.askWrite ?? true);
  let drawPost = $state(false);
  let drawWrite = $state(false);
  let query = $state("");
  let helperError = $state<string | null>(null);
  let busy = $state(false);
  let browserOpens = $state(0);
  let browserUrl = $state("");
  let lookEpoch = $state(0);
  let nodEpoch = $state(0);
  let settleEpoch = $state(0);
  let settling = $state(false);

  let stopHandoff: HandoffListen["stop"] | null = null;
  let listenGen = 0;
  let advanceTimer = 0;
  let lookReady = false;
  let looks = 0;

  const title = $derived(stepTitle(step));
  const countLabel = $derived(`${stepNumber(step)} of 4`);
  const listed = $derived(visibleRepos(repos, query));
  const reopenUrl = $derived(handoffUrl ?? devicePageUrl());
  const canGoBack = $derived((step === "helper" && elsewhere) || previousStep(step, helperSkipped) !== null);
  const watchLabel = $derived(picked ? `Start watching ${picked}` : "Start watching");

  function current(): FirstRunRecord {
    return {
      version: 1,
      completed: false,
      step,
      helperSkipped,
      github,
      handoffUrl,
      login,
      repo: picked,
      address,
      elsewhere,
      askPost,
      askWrite,
    };
  }

  function persist(): void {
    saveRecord(current());
  }

  function openBrowser(url: string): void {
    browserOpens += 1;
    browserUrl = url;
    browserPort().openExternal(url);
  }

  function syncListen(): void {
    listenGen += 1;
    const gen = listenGen;
    stopHandoff?.();
    stopHandoff = null;
    if (step !== "github" || github !== "waiting") {
      return;
    }
    const pending = consumeHandoff();
    if (pending) {
      applyGithub(pending);
      return;
    }
    const session = listenForHandoff();
    stopHandoff = session.stop;
    void session.done.then((outcome) => {
      if (gen !== listenGen) {
        return;
      }
      applyGithub(outcome);
    });
  }

  function prefersReducedMotion(): boolean {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  }

  function clearAdvance(): void {
    window.clearTimeout(advanceTimer);
    advanceTimer = 0;
  }

  function applyGithub(outcome: HandoffPayload): void {
    stopHandoff?.();
    stopHandoff = null;
    if (outcome.type === "cancelled") {
      github = "idle";
      persist();
      return;
    }
    login = outcome.login;
    github = "connected";
    nodEpoch += 1;
    persist();
    clearAdvance();
    const wait = prefersReducedMotion() ? 0 : 640;
    advanceTimer = window.setTimeout(() => {
      advanceTimer = 0;
      if (step !== "github" || github !== "connected") {
        return;
      }
      step = "repo";
      persist();
    }, wait);
  }

  function connect(): void {
    if (busy || github === "waiting") {
      return;
    }
    localStorage.removeItem(HANDOFF_STORAGE_KEY);
    github = "waiting";
    handoffUrl = devicePageUrl();
    persist();
    syncListen();
    openBrowser(handoffUrl);
  }

  function reopen(event: MouseEvent): void {
    event.preventDefault();
    const url = handoffUrl ?? devicePageUrl();
    handoffUrl = url;
    if (github !== "waiting") {
      github = "waiting";
    }
    persist();
    syncListen();
    openBrowser(url);
  }

  function back(): void {
    clearAdvance();
    helperError = null;
    if (step === "helper" && elsewhere) {
      elsewhere = false;
      persist();
      return;
    }
    const prev = previousStep(step, helperSkipped);
    if (!prev) {
      return;
    }
    step = prev;
    persist();
    syncListen();
  }

  async function startHelper(): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    helperError = null;
    const result = await helperPort().start(address);
    busy = false;
    if (result === "started") {
      helperSkipped = false;
      elsewhere = false;
      step = "github";
      persist();
      return;
    }
    helperError = helperMissCopy(address);
  }

  async function checkAddress(): Promise<void> {
    if (busy) {
      return;
    }
    const next = normalizeAddress(address);
    if (!next) {
      helperError = "That address doesn't look like a host and port. Nothing is running yet.";
      return;
    }
    address = next;
    busy = true;
    helperError = null;
    const found = await helperPort().find(address);
    busy = false;
    if (found) {
      helperSkipped = false;
      elsewhere = false;
      step = "github";
      persist();
      return;
    }
    helperError = helperMissCopy(address);
  }

  function showElsewhere(): void {
    elsewhere = true;
    helperError = null;
    persist();
  }

  function pick(repo: RepoChoice): void {
    if (!canPick(repo)) {
      return;
    }
    picked = repo.fullName;
    persist();
  }

  function goRepo(): void {
    step = "repo";
    persist();
  }

  function continueRepo(): void {
    if (!picked) {
      return;
    }
    step = "rules";
    persist();
  }

  function togglePost(next: boolean): void {
    askPost = next;
    drawPost = next;
    persist();
  }

  function toggleWrite(next: boolean): void {
    askWrite = next;
    drawWrite = next;
    persist();
  }

  function openFix(event: MouseEvent, fullName: string): void {
    event.preventDefault();
    openBrowser(fixAccessHref(fullName));
  }

  function finish(): void {
    if (!picked || settling) {
      return;
    }
    settling = true;
    settleEpoch += 1;
    const wait = prefersReducedMotion() ? 0 : 920;
    window.setTimeout(() => {
      const done = current();
      done.completed = true;
      done.step = "rules";
      saveRecord(done);
      onDone();
    }, wait);
  }

  function markHelper(node: HTMLElement): () => void {
    node.ownerDocument.defaultView?.sessionStorage.setItem("dasdevbot.helper-painted", "1");
    return () => {};
  }

  $effect(() => {
    if (phase !== "flow") {
      return;
    }
    const currentStep = step;
    if (!lookReady) {
      lookReady = true;
      return;
    }
    void currentStep;
    looks += 1;
    lookEpoch = looks;
  });

  onMount(() => {
    if (saved) {
      syncListen();
      return () => {
        listenGen += 1;
        clearAdvance();
        stopHandoff?.();
      };
    }
    let gone = false;
    void helperPort()
      .find(address)
      .then((found) => {
        if (gone) {
          return;
        }
        if (found) {
          helperSkipped = true;
          step = "github";
        } else {
          step = "helper";
        }
        phase = "flow";
        persist();
      })
      .catch(() => {
        if (gone) {
          return;
        }
        step = "helper";
        phase = "flow";
        helperError = helperMissCopy(address);
        persist();
      });
    return () => {
      gone = true;
      listenGen += 1;
      clearAdvance();
      stopHandoff?.();
    };
  });
</script>

{#if phase === "checking"}
  <div class="blank" data-first-run data-phase="checking" data-watching="off" data-step="check"></div>
{:else}
  <div
    class="screen"
    data-first-run
    data-phase="flow"
    data-step={step}
    data-watching="off"
    data-browser-opens={browserOpens}
    data-browser-url={browserUrl}
    data-helper-skipped={helperSkipped ? "yes" : "no"}
    data-looks={lookEpoch}
  >
    <div class="column">
      <header class="top">
        <p class="brand">dasdevbot</p>
        <p class="count">{countLabel}</p>
      </header>

      <div class="skip-row">
        {#if step === "github" && helperSkipped}
          <p data-helper-skip>{helperSkipCopy(address)}</p>
        {/if}
      </div>

      <div class="title-row">
        <GlyphSlot>
          {#snippet glyph()}
            <Curl {lookEpoch} {nodEpoch} {settleEpoch} blink />
          {/snippet}
        </GlyphSlot>
        <h1 id="step-title">{title}</h1>
      </div>

      {#if step === "helper"}
        <div {@attach markHelper}>
          <p class="lead">It stays on this machine and wakes Reviewer when the repo changes.</p>
          {#if elsewhere}
            <section class="devices" aria-labelledby="devices-label">
              <h2 id="devices-label">Devices / Advanced</h2>
              <label class="field" for="daemon-address">Daemon address</label>
              <input id="daemon-address" bind:value={address} autocomplete="off" spellcheck="false" onchange={persist} />
            </section>
          {:else}
            <button class="link aside" type="button" onclick={showElsewhere}>I run it elsewhere</button>
          {/if}
          {#if helperError}
            <div class="banner" role="status">
              <p>{helperError}</p>
              <button class="link" type="button" onclick={() => void (elsewhere ? checkAddress() : startHelper())}>
                Retry now
              </button>
            </div>
          {/if}
        </div>
      {:else if step === "github"}
        {#if github === "waiting"}
          <p class="lead">Finish in your browser. This window will continue on its own.</p>
          <a class="link aside" href={reopenUrl} target="_blank" rel="noopener noreferrer" onclick={reopen}>Open again</a>
        {:else if github === "connected" && login}
          <p class="lead">{connectedCopy(login)}</p>
        {:else if github === "connected"}
          <p class="lead">GitHub is connected. Pick the repo next.</p>
        {:else}
          <p class="lead">{GITHUB_LEAD}</p>
        {/if}
      {:else if step === "repo"}
        <p class="lead">{REPO_LEAD}</p>
        <label class="field" for="repo-search">Repos</label>
        <input id="repo-search" type="search" placeholder="Search repos" bind:value={query} autocomplete="off" />
        {#if query.trim() && listed.length === 0}
          <p class="empty" role="status">{emptyRepoCopy(query)}</p>
        {:else}
          <div class="rows" role="radiogroup" aria-label="Repos">
            {#each listed as repo (repo.fullName)}
              {@const allowed = canPick(repo)}
              <div class="row-line">
                <button
                  class="row"
                  type="button"
                  role="radio"
                  aria-checked={picked === repo.fullName}
                  disabled={!allowed}
                  onclick={() => pick(repo)}
                >
                  <span class="repo-name">
                    {#each highlightParts(repo.fullName, query) as part, index (`${repo.fullName}-${index}`)}
                      {#if part.match}<strong>{part.text}</strong>{:else}{part.text}{/if}
                    {/each}
                  </span>
                  {#if allowed}
                    <span class="branch">{repo.defaultBranch}</span>
                  {:else}
                    <span class="needs">needs access</span>
                  {/if}
                </button>
                {#if !allowed}
                  <a
                    class="link fix"
                    href={fixAccessHref(repo.fullName)}
                    target="_blank"
                    rel="noopener noreferrer"
                    onclick={(event) => openFix(event, repo.fullName)}
                  >
                    Fix on GitHub
                  </a>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      {:else}
        <ul class="rules">
          <li>
            <PenCheckbox
              label="Posting to GitHub"
              checked={askPost}
              draw={drawPost}
              teammates={["Reviewer"]}
              onToggle={togglePost}
            />
          </li>
          <li>
            <PenCheckbox
              label="Writing outside the repo"
              checked={askWrite}
              draw={drawWrite}
              teammates={["Reviewer"]}
              onToggle={toggleWrite}
            />
          </li>
          <li>
            <PenCheckbox
              label="Anything destructive"
              checked={true}
              disabled={true}
              note="off in this build"
              teammates={["Reviewer"]}
            />
          </li>
        </ul>
        <p class="reassure">Nothing leaves this machine without your OK. You can loosen this later, one rule at a time.</p>
      {/if}

      <div class="footer">
        <button class="back" type="button" disabled={!canGoBack} onclick={back}>Back</button>
        {#if step === "helper"}
          <button
            class="primary"
            type="button"
            disabled={busy}
            onclick={() => void (elsewhere ? checkAddress() : startHelper())}
          >
            {elsewhere ? "Check this address" : "Start helper"}
          </button>
        {:else if step === "github"}
          {#if github === "connected"}
            <button class="primary" type="button" onclick={goRepo}>Continue</button>
          {:else if github !== "waiting"}
            <button class="primary" type="button" onclick={connect}>Connect GitHub</button>
          {/if}
        {:else if step === "repo"}
          <button class="primary" type="button" disabled={!picked} onclick={continueRepo}>Continue</button>
        {:else}
          <button class="primary" type="button" disabled={!picked || settling} onclick={finish}>{watchLabel}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .blank,
  .screen {
    min-height: 100vh;
    background: var(--paper-base);
    color: var(--ink-1);
  }

  .screen {
    padding: var(--s-8) var(--s-6);
  }

  .column {
    width: min(640px, 100%);
    margin: 0 auto;
  }

  .top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s-4);
    padding-bottom: var(--s-4);
    border-bottom: 1px solid var(--hairline);
  }

  .brand {
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  .count {
    font-family: var(--font-machine);
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .skip-row {
    height: 18px;
    margin-top: var(--s-4);
    color: var(--ink-3);
    font-size: var(--t-micro);
    line-height: 18px;
    white-space: nowrap;
  }

  .title-row {
    display: flex;
    align-items: flex-start;
    gap: var(--s-4);
    margin-top: var(--s-4);
  }

  h1 {
    font-size: var(--t-display);
    line-height: var(--lh-display);
    font-weight: var(--w-semibold);
    letter-spacing: var(--track-tight);
  }

  .lead,
  .reassure,
  .empty {
    margin-top: var(--s-4);
    max-width: 38rem;
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .reassure {
    margin-top: var(--s-5);
  }

  .link {
    color: var(--accent);
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  .aside {
    display: inline-block;
    margin-top: var(--s-4);
  }

  .devices {
    margin-top: var(--s-5);
    padding-top: var(--s-4);
    border-top: 1px solid var(--hairline);
  }

  h2,
  .field {
    color: var(--ink-2);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    font-weight: var(--w-semibold);
  }

  .field {
    display: block;
    margin-top: var(--s-3);
    margin-bottom: var(--s-2);
  }

  input {
    width: 100%;
    height: 40px;
    padding: 0 var(--s-3);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--r-sm);
    background: var(--paper-sunken);
    color: var(--ink-1);
    font: inherit;
  }

  .banner {
    margin-top: var(--s-4);
    padding: var(--s-3);
    border: 1px solid var(--hairline);
    border-radius: var(--r-sm);
    background: var(--paper-sunken);
  }

  .banner p {
    color: var(--ink-2);
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .banner .link {
    margin-top: var(--s-2);
  }

  .rows {
    margin-top: var(--s-4);
    border-top: 1px solid var(--hairline);
  }

  .row-line {
    border-bottom: 1px solid var(--hairline);
  }

  .row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s-4);
    width: 100%;
    padding: var(--s-3) var(--s-2);
    border: 0;
    background: transparent;
    color: var(--ink-1);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .row[aria-checked="true"] {
    background: var(--accent-bg);
  }

  .row:disabled {
    color: var(--ink-2);
    cursor: not-allowed;
  }

  .repo-name {
    font-size: var(--t-body);
    line-height: var(--lh-body);
  }

  .branch,
  .needs {
    flex: none;
    font-size: var(--t-micro);
    line-height: var(--lh-micro);
    color: var(--ink-3);
  }

  .branch {
    font-family: var(--font-machine);
  }

  .fix {
    display: inline-block;
    margin: 0 0 var(--s-3) var(--s-2);
    font-size: var(--t-meta);
  }

  .rules {
    margin: var(--s-5) 0 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--hairline);
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    margin-top: var(--s-8);
    padding-top: var(--s-4);
    border-top: 1px solid var(--hairline);
  }

  .back {
    border: 0;
    background: none;
    padding: 0;
    color: var(--ink-1);
    font: inherit;
    cursor: pointer;
  }

  .back:disabled {
    color: var(--ink-3);
    cursor: not-allowed;
  }

  .primary {
    height: 40px;
    padding: 0 var(--s-4);
    border: 0;
    border-radius: var(--r-md);
    background: var(--ink-1);
    color: var(--paper-raised);
    font-size: var(--t-body);
    font-weight: var(--w-semibold);
    cursor: pointer;
  }

  .primary:active {
    background: var(--ink-press);
  }

  .primary:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  @media (prefers-reduced-motion: reduce) {
    .primary:active,
    .back:active,
    .row:active {
      transition: none;
    }
  }
</style>
