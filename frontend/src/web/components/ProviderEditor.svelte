<script lang="ts">
  import { onMount } from "svelte";
  import type { ProviderDraft, ProviderProfile } from "../lib/types";

  type Props = {
    provider?: ProviderProfile | null;
    models?: string[];
    busy?: boolean;
    error?: string;
    onSave: (draft: ProviderDraft) => void;
    onDelete: () => void;
    onRefreshModels: () => void;
    onClose: () => void;
  };

  let {
    provider = null,
    models = [],
    busy = false,
    error = "",
    onSave,
    onDelete,
    onRefreshModels,
    onClose,
  }: Props = $props();

  const blankDraft = (): ProviderDraft => ({
    id: "",
    name: "",
    type: "openai",
    url: "",
    key: "",
    timeout_seconds: "30",
  });

  let draft = $state<ProviderDraft>(blankDraft());

  onMount(() => {
    draft = provider
      ? { ...provider, key: "", timeout_seconds: String(provider.timeout_seconds) }
      : blankDraft();
  });

  function submit() { onSave(draft); }
</script>

<aside class="editor" aria-label="Provider editor">
  <header><h2>{provider ? `Edit ${provider.name}` : "New provider"}</h2><button class="icon" onclick={onClose}>×</button></header>
  <form onsubmit={(event) => { event.preventDefault(); submit(); }}>
    <label>Profile ID<input bind:value={draft.id} disabled={provider !== null} required pattern="[A-Za-z0-9_-]+" /></label>
    <label>Display name<input bind:value={draft.name} required /></label>
    <label>Provider type<select bind:value={draft.type}><option value="openai">OpenAI compatible</option><option value="google">Google GenAI</option><option value="anthropic">Anthropic</option><option value="ollama">Ollama</option></select></label>
    <label>Endpoint<input bind:value={draft.url} required placeholder="https://api.example.com/v1" /></label>
    <label>API key<input bind:value={draft.key} type="password" placeholder={provider?.key_configured ? "Stored key (leave blank to keep)" : "Required for remote providers"} /></label>
    <label>Timeout (seconds)<input bind:value={draft.timeout_seconds} inputmode="numeric" required /></label>
    {#if error}<p class="error">{error}</p>{/if}
    <footer><button class="primary" disabled={busy}>Save profile</button>{#if provider}<button type="button" onclick={onRefreshModels} disabled={busy}>Refresh models</button>{/if}</footer>
  </form>
  {#if provider}<section class="models"><h3>Discovered models</h3>{#if models.length}<ul>{#each models as model (model)}<li>{model}</li>{/each}</ul>{:else}<p>No cached models. Refresh after testing the connection.</p>{/if}</section><button class="danger" onclick={onDelete} disabled={busy}>Delete provider</button>{/if}
</aside>
