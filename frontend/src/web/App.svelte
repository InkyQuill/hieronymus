<script lang="ts">
  const route = window.location.pathname;
  const active = route === "/admin" ? "Admin" : "Providers";
</script>

<main>
  <aside>
    <h1>Hieronymus</h1>
    <p>Local configuration</p>
    <nav>
      <a href="/admin">Overview</a>
      <a class:active={active === "Providers"} href="/config">Providers</a>
      <a href="/config/dreaming">Dreaming</a>
      <a href="/config/ingest">Ingest</a>
      <a href="/config/release">Release</a>
    </nav>
  </aside>
  <section>
    <h2>{active}</h2>
    <p>The local web console is starting.</p>
  </section>
</main>
