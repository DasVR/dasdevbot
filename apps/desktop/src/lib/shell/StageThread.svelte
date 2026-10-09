<script lang="ts">
  import { CHEVRON_PATH } from "../pen";
  import WaitingStep from "./WaitingStep.svelte";

  const replyLead = "Found one thing worth raising. If ";
  const replyMid = " rejects on a 401, the handoff lock is never released, so the next session can’t take it. I drafted one comment suggesting a ";
  const replyEnd = ". Posting to GitHub needs your OK.";
</script>

{#snippet reviewerTurn(when: string, companion: boolean)}
  <section class="turn">
    <div class="who">
      <span class="mg" aria-hidden="true">R</span>
      <b>Reviewer</b>
      <span>{when}</span>
    </div>
    {#if !companion}
      <div class="tools">
        <div class="sumwrap">
          <button class="sum" type="button" aria-expanded="false">
            <svg class="chev" viewBox="0 0 12 12" aria-hidden="true">
              <path d={CHEVRON_PATH} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <span class="n">Used 5 tools</span>
            <span class="mono">· 42s</span>
          </button>
        </div>
      </div>
    {/if}
    <div class="bub-bot">
      <p>
        {replyLead}<code>refresh()</code>{replyMid}<code>try/finally</code>{replyEnd}
      </p>
    </div>
    <div class="tools wait">
      <WaitingStep id="stage" say="Posting the comment to DasVR/NIL #212" meta="gh pr comment 212 · ev_3f9a2c · external" />
    </div>
  </section>
{/snippet}

<div class="stage-thread">
  <div class="col">
    <p class="day">Today · 10:38 AM</p>
    <section class="turn">
      <div class="who">
        <span class="mg" aria-hidden="true">R</span>
        <b>Reviewer</b>
        <span>10:38 AM</span>
      </div>
      <div class="bub-bot">
        <p>
          Woke on push <code>a41c9e2</code> to DasVR/NIL · phase0, “handoff: release lock on refresh”. Three files changed. Say the word and I’ll review it.
        </p>
      </div>
    </section>
    <div class="me">
      <div class="bub-me">
        <div class="fill"></div>
        <p class="txt">Pushed. Review the handoff fix, one comment max.</p>
      </div>
      <span class="meta-t">10:42 AM</span>
    </div>
    {@render reviewerTurn("10:42 AM", false)}
  </div>
  <div class="cstream">
    {@render reviewerTurn("10:42 AM", true)}
  </div>
</div>

<style>
  .stage-thread {
    position: absolute;
    inset: 0;
  }

  .col {
    position: absolute;
    left: 50%;
    width: 640px;
    margin-left: -320px;
    top: 34px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .cstream {
    position: absolute;
    inset: 0;
    padding: 18px 18px 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    visibility: hidden;
  }

  :global(.win[data-shell-form="companion"]) .col {
    visibility: hidden;
  }

  :global(.win[data-shell-form="companion"]) .cstream {
    visibility: visible;
  }

  .day {
    align-self: center;
    font: var(--t-micro) / var(--lh-micro) var(--font-machine);
    color: var(--ink-3);
  }

  .me {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 5px;
  }

  .bub-me {
    position: relative;
    max-width: 480px;
    padding: 10px 16px 11px;
    border-radius: var(--r-lg);
  }

  .bub-me > .fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: var(--paper-sunken);
  }

  .bub-me > .txt {
    position: relative;
  }

  .meta-t {
    font: var(--t-micro) / var(--lh-micro) var(--font-machine);
    color: var(--ink-3);
    padding: 0 8px;
  }

  .turn {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }

  .who {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 20px;
  }

  .mg {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--convex), var(--paper-sunken);
    display: grid;
    place-items: center;
    font: var(--w-semibold) 11px / 1 var(--font-ui);
    color: var(--ink-2);
    box-shadow: var(--highlight-top);
  }

  .who b {
    font: var(--w-semibold) var(--t-meta) / var(--lh-meta) var(--font-ui);
  }

  .who span {
    font: var(--t-micro) / var(--lh-micro) var(--font-machine);
    color: var(--ink-3);
  }

  .bub-bot {
    margin-left: 28px;
    max-width: 560px;
    padding: 15px 20px 16px;
    border-radius: var(--r-lg);
    background: var(--convex), var(--paper-raised);
  }

  .bub-bot code {
    font-family: var(--font-machine);
    font-size: 12.5px;
    background: var(--paper-sunken);
    border-radius: var(--r-xs);
    padding: 1px 5px;
  }

  .tools {
    margin-left: 28px;
    width: 520px;
  }

  .sumwrap {
    overflow: hidden;
    height: 30px;
  }

  .sum {
    height: 30px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 6px;
    margin-left: -6px;
    border: 0;
    background: none;
    cursor: pointer;
    border-radius: var(--r-md);
    color: var(--ink-2);
    font: var(--w-medium) var(--t-meta) / 1 var(--font-ui);
  }

  .chev {
    width: 12px;
    height: 12px;
  }

  .mono {
    font-size: var(--t-micro);
    color: var(--ink-3);
  }

  .cstream .bub-bot {
    margin-left: 0;
    max-width: none;
    padding: 13px 16px 14px;
    font-size: 14px;
    line-height: 21px;
  }

  .cstream .tools {
    margin-left: 0;
    width: auto;
  }

  .cstream .tools.wait {
    max-width: 364px;
  }
</style>
