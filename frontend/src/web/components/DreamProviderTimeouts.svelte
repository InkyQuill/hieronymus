<script lang="ts">
  import { listProviders, saveProvider } from "../lib/api";
  import type { ProviderProfile } from "../lib/types";

  let { providers, providerIds }: { providers: ProviderProfile[]; providerIds: string[] } = $props();
  let timeouts = $state<Record<string, number | undefined>>({});
  let confirmed = $state<Record<string, { before: number; after: number }>>({});
  let busy = $state<Record<string, boolean>>({});
  let errors = $state<Record<string, string>>({});
  let saved = $state<Record<string, boolean>>({});
  const assigned = $derived(providers.filter(provider => providerIds.includes(provider.id)));
  function timeoutFor(provider: ProviderProfile) {
    const previous = confirmed[provider.id];
    return timeouts[provider.id] ?? (previous?.before === provider.timeout_seconds ? previous.after : provider.timeout_seconds);
  }

  async function save(id: string) {
    const displayed = providers.find(provider => provider.id === id);
    if (!displayed) return;
    const timeout = timeoutFor(displayed);
    busy[id] = true; errors[id] = ""; saved[id] = false;
    try {
      // Read the current profile so this control changes only its timeout.
      const profile = (await listProviders()).find(provider => provider.id === id);
      if (!profile) throw new Error("This provider no longer exists. Reload Dreaming settings.");
      const result = await saveProvider({
        id, name: profile.name, type: profile.type, url: profile.url, key: "",
        timeout_seconds: String(timeout),
        context_window: profile.context_window == null ? "" : String(profile.context_window),
      });
      confirmed[id] = { before: displayed.timeout_seconds, after: result.timeout_seconds };
      delete timeouts[id];
      saved[id] = true;
    } catch (reason) {
      errors[id] = reason instanceof Error ? reason.message : "Could not save the request timeout.";
    } finally { busy[id] = false; }
  }
</script>

{#if assigned.length}
  <section class="mt-8 border-t border-default pt-6" aria-labelledby="dream-timeouts-title">
    <h2 id="dream-timeouts-title" class="text-h3">Dreaming request timeouts</h2>
    <p class="mt-2 max-w-[70ch] text-body-sm text-secondary">Set how long to wait for each AI request. For DeepSeek, start with 300 seconds; longer timeouts are supported. These settings are shared with other tasks using the same provider profile. Memory comparison has its own timeout below. Saved changes apply to the next run.</p>
    {#each assigned as provider (provider.id)}
      <form class="mt-4 flex flex-wrap items-end gap-3" onsubmit={event => { event.preventDefault(); void save(provider.id); }}>
        <label class="grid gap-1.5 text-caption text-secondary">{provider.name} timeout (seconds)
          <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary" type="number" min="0" step="any" required value={timeoutFor(provider)} oninput={event => { timeouts[provider.id] = event.currentTarget.valueAsNumber; saved[provider.id] = false; }} disabled={busy[provider.id]} />
        </label>
        <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm text-accent-text disabled:opacity-60" disabled={busy[provider.id] || !(timeoutFor(provider) > 0)}>Save {provider.name} timeout</button>
        {#if errors[provider.id]}<p role="alert" class="w-full text-body-sm text-danger">{errors[provider.id]}</p>{/if}
        {#if saved[provider.id]}<p role="status" class="w-full text-body-sm text-secondary">{provider.name} timeout saved.</p>{/if}
      </form>
    {/each}
  </section>
{/if}
