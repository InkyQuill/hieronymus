<script lang="ts">
  let { section }: { section: string } = $props();
  const sections = [
    { id: "providers", label: "AI providers", href: "/config" },
    { id: "dreaming", label: "Dreaming", href: "/config/dreaming" },
    { id: "ingest", label: "Incoming memory", href: "/config/ingest" },
    { id: "release", label: "Updates", href: "/config/release" },
  ];
</script>

<nav class="mb-6 flex flex-wrap gap-1 border-b border-default pb-3" aria-label="Configuration sections">
  {#each sections as item (item.id)}
    <a class="inline-flex min-h-11 items-center rounded-sm px-3 py-2 text-body-sm no-underline {section === item.id ? 'bg-raised text-primary' : 'text-secondary hover:bg-raised hover:text-primary'}" href={item.href} aria-current={section === item.id ? "page" : undefined}>{item.label}</a>
  {/each}
</nav>
