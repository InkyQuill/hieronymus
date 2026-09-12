<script lang="ts">
  import { prepareAgentConnection } from "../lib/api";
  import type { AgentConnection } from "../lib/api";
  let agent = $state("codex");
  let connections = $state.raw<AgentConnection[]>([]);
  let busy = $state(false);
  let message = $state("");
  let error = $state("");
  const selected = $derived(connections.find((entry) => entry.id === agent));
  async function prepare() {
    busy = true;
    error = "";
    message = "";
    try { connections = await prepareAgentConnection(); }
    catch (reason) { error = reason instanceof Error ? reason.message : "Could not prepare the connection. Try again."; }
    finally { busy = false; }
  }
  async function copy() {
    if (!selected) return;
    error = "";
    try {
      await navigator.clipboard.writeText(selected.instructions);
      message = `Copied. Paste the request into ${selected.name} with your writing project open.`;
    } catch {
      error = "Copying is unavailable in this browser. Open the setup request below and copy its text.";
    }
  }
</script>

<section class="mx-auto grid max-w-4xl gap-8" aria-labelledby="connect-heading">
  <header>
    <p class="mb-3 text-eyebrow uppercase tracking-[0.16em] text-accent-text">Your writing companion</p>
    <h1 id="connect-heading" class="text-display">Connect your agent</h1>
    <p class="mt-4 max-w-2xl text-body text-secondary">Add the Hieronymus MCP connection and skills to Codex, Cowork, or pi. The connection gives your agent access to memory; the skills teach it when and how to use that memory for your writing project.</p>
  </header>
  <ol class="grid gap-4">
    <li class="rounded-md border border-default bg-surface p-6">
      <h2 class="text-h3">1. Choose where you work</h2>
      <fieldset class="mt-4 flex flex-wrap gap-3">
        <legend class="sr-only">Writing agent</legend>
        {#each [{ id: "codex", name: "Codex" }, { id: "cowork", name: "Cowork" }, { id: "pi", name: "pi" }] as option (option.id)}
          <label class="flex min-h-11 cursor-pointer items-center gap-2 rounded-sm border border-default px-4 py-2 text-body"><input type="radio" name="agent" value={option.id} bind:group={agent} onchange={() => { message = ""; }} />{option.name}</label>
        {/each}
      </fieldset>
      {#if !connections.length}<button class="mt-5 min-h-11 rounded-sm border border-accent bg-raised px-5 py-2 text-body-sm font-medium text-accent-text disabled:opacity-60" onclick={prepare} disabled={busy}>{busy ? "Preparing…" : "Prepare connection"}</button>{/if}
    </li>
    <li class="rounded-md border border-default bg-surface p-6">
      <h2 class="text-h3">2. Add the MCP connection and skills</h2>
      <p class="mt-3 text-body text-secondary">Open your writing project in your agent. Copy and paste the setup request to install both the memory connection and the Hieronymus skills. Follow the permission prompts your agent shows.</p>
      <button class="mt-5 min-h-11 rounded-sm border border-accent bg-raised px-5 py-2 text-body-sm font-medium text-accent-text disabled:opacity-60" onclick={copy} disabled={!selected}>Copy setup request</button>
      {#if selected}<details class="mt-4 text-body-sm text-secondary"><summary class="cursor-pointer py-2">Read the setup request</summary><textarea class="mt-2 min-h-52 w-full rounded-sm border border-default bg-raised p-3 text-body-sm text-primary" aria-label="Setup request" readonly value={selected.instructions}></textarea></details>{/if}
    </li>
    <li class="rounded-md border border-default bg-surface p-6">
      <h2 class="text-h3">3. Continue writing</h2>
      <p class="mt-3 text-body text-secondary">Once your agent confirms that the MCP connection works and the skills are available, work with it as usual. Return here to check what it remembers, add pointers, or flag memories that are stale or wrong.</p>
      <p class="mt-3 text-body-sm text-secondary">Preparing or copying a request does not verify the connection. Your agent checks it from the environment where your project is open.</p>
    </li>
  </ol>
  {#if message}<p role="status" class="text-body text-success">{message}</p>{/if}
  {#if error}<p role="alert" class="text-body text-danger">{error}</p>{/if}
  <p class="text-body-sm text-secondary">Already connected? <a class="text-accent-text underline" href="/admin/memory">Browse memory</a> or <a class="text-accent-text underline" href="/config">open settings</a>.</p>
</section>
