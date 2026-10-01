<script lang="ts">
  import { setSecret } from "./api";

  let handle = $state("");
  let draft = $state("");
  let last4 = $state("");
  let error = $state("");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const value = draft;
    draft = "";
    const name = handle.trim();
    try {
      const stored = await setSecret(name, value);
      last4 = stored.last4;
      error = "";
    } catch (err) {
      last4 = "";
      error = err instanceof Error ? err.message : "secret entry failed";
    }
  }
</script>

<form class="secret-entry" onsubmit={submit}>
  <label>
    Handle
    <input name="handle" autocomplete="off" bind:value={handle} />
  </label>
  <label>
    Secret
    <input name="secret" type="password" autocomplete="off" bind:value={draft} />
  </label>
  <button type="submit">Save</button>
  {#if last4}
    <p>Stored. Last 4: {last4}</p>
  {/if}
  {#if error}
    <p>{error}</p>
  {/if}
</form>
